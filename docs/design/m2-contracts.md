# Hikasa M2 — Binding Contracts

> **Status: binding** for milestone M2 (version 0.2, `docs/16-roadmap.md`), branch `feat/m2`. It **extends**
> `docs/design/m1-contracts.md`: every tenet (T1–T11), decision (D-01…D-58), contract and test rule of M1 stays
> in force unless a decision below revises it with a new id. The plan, the scope and the maintainer's answers are
> in `docs/design/m2-plan.md` (§4.1); where the plan and this document disagree, this document wins.
>
> The contract is written **wave by wave** (plan §2): each wave's section is completed before the wave starts, so
> its implementers code against settled types. Sections of later waves are outlines until then (plan §6).

## Table of contents

- §0 Ground rules: scope, new tenets, decision register D-59…D-212, dependencies, documentation updates
- §1 Wave contracts — §1.0 W0 Infrastructure (complete) · §1.1 W1 Settings store · §1.2 W2 Sessions, workspaces, packages ·
  §1.3 W3 VFS mutations, clipboard, generic copy · §1.4 W4 GUI shell (parts W4a Shell, W4b Sessions, W4c Settings
  dialogs) · §1.5 W5 Archives (parts W5a Read, W5b Write) · §1.6 W6 Folder operations and sync engine (parts W6a
  Foundations, W6b Operations, W6c Sync, W6d GUI hooks) · §1.7 W7 Reports (parts W7a Core, W7b Service and CLI) ·
  §1.8–§1.14 W8–W14 (outline in plan §6, written before each wave)
- §6 Tests and acceptance per wave — §6.0 W0 · §6.1 W1 · §6.2 W2 · §6.3 W3 · §6.4 W4 · §6.5 W5 · §6.6 W6 · §6.7 W7

---

## 0. Ground rules

### 0.1 Scope

The P0 ids of plan §1, minus the deferrals of Q1 (plan §4.1). Release: 0.2.0 after W14.

### 0.2 Tenets added in M2

| Id | Tenet | Enforced by |
|---|---|---|
| T12 | **Plan, confirm, execute.** Every mutation (file operation, sync, archive rebuild, save, settings import) is a serializable plan computed by core, confirmed by the adapter and executed by core with per-item results; `--dry-run` prints the plan that would be executed. | W3, W5, W6, W10 contracts; property tests |
| T13 | **Persisted state is versioned and program-owned.** Every settings file has `schema_version`, a migration chain, unknown-key preservation (Q11) and an atomic write. | W1 contract; round-trip properties |
| T14 | **Generated references are never edited by hand.** `docs/reference/commands.md` is the output of `cargo xtask commands-doc` over `core/hikasa-session/catalogue/commands.toml`; CI fails when it is stale. | CI `docs` job (D-66) |

### 0.3 Decision register (M2)

| ID | Decision | Rationale | Docs |
|---|---|---|---|
| D-59 | Q1 = a: the deferrals of plan §4 Q1 move to M3 (merge parts, snapshot/remote/profile parts, built-in document converters, spreadsheet/HTML table extraction, DDS, ACL and 8.3 options, taskbar previews, "update available", the third-party highlighter fallback). | Maintainer, 2026-10-28. | 16 |
| D-60 | Q2 = a: Folder Sync P0 (FS-001/002/004/005) is in M2: engine W6, view W9. | Maintainer. | 16 |
| D-61 | Q3 = a: `docs/15` options get ids `OPT-001…`, one per page or setting group, each with a priority. | A1: OPT had no ids. | 15 |
| D-62 | Q4 = a, Q5 = a, Q6 = a: configuration changes are detected by polling (window focus and every 2 s); browse dialogs are an in-house VFS browser; Print = the HTML report in the default browser. No `notify`, no `rfd`. | Maintainer. | 03, 13, 14 |
| D-63 | Q8 = a, Q9 = **b**, Q10 = b: archives are read in every P0 format and written for zip and the tar family by full rebuild; xz uses the pure-Rust `lzma-rust2` (no `liblzma`, no C code for LZMA); `thread-priority` is approved, `uzers` is not (owner and group stay numeric). Approved crates: `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `zstd`, `image`, `arboard`, `trash`, `open`, `ctrlc`, `thread-priority`. Each is added in the wave that first uses it, with its `docs/02` line and fixed features. | Maintainer, 2026-10-28. | 02 |
| D-64 | Q11 = a, Q12 = a, Q13 = a: program-owned TOML keeps unknown keys, not comments; CI uses nextest and the docs checks, benches run in a manual job (25 % tolerance); help topics are Markdown files in `help/`, one per CLI subcommand, view and dialog; the localization table (ARCH-006) stays P1. | Maintainer. | 02, 17 |
| D-65 | **Command catalogue in core** (Q7 = a). The data of every command of `reference/commands.md` (458 rows, 14 sections) lives in `core/hikasa-session/catalogue/commands.toml`, embedded and parsed by `hikasa_session::commands`. Entry: `id`, `menu`, `caption`, `shortcut` (reference spelling), `description`, `status` (`implemented`/`planned`); section: `key`, `title`, `help` topic, optional `note`. The GUI keeps presentation and availability (toolbar labels, radio groups, predicates) next to its view models; its tests require that every command it executes is an `implemented` entry of its view type's sections with the same default shortcut, and that every `implemented` entry of those sections is executed by it. | ARCH-070 "shared with the CLI"; core holds data only, no GUI type. | 01 §3.6, 14 |
| D-66 | **Generated reference.** `docs/reference/commands.md` = `Catalogue::render_markdown(true)` (the M1 table plus a Status column). `cargo xtask commands-doc --check` and `cargo xtask help-check` run in the CI `docs` job. Without the Status column the generated file equals the M1 hand-maintained file apart from its final blank line (checked when W0 was written). | docs/17 §4.2; T14. | 17 |
| D-67 | **Help topics.** `help/<id>.md`, first line `# <title>`, links `[label](topic:<id>)`, embedded by `hikasa_session::help`. `hikasa help [words…]` joins the words with `-` and falls back to the leading words (`format list` → `format`); an unknown topic exits 100 with up to five suggestions (substring or edit distance ≤ 2). `hikasa help --list`. `hikasa.help.v1` = `{topic: {id, title, markdown, links} or null, topics: [{id, title}]}`. `help` reads no configuration. In the GUI, `ContextHelp` (F1) shows the topic of the active view type (its first catalogue section's `help`), `HelpContents` the index, in a resizable right-hand panel; links open topics; Esc closes it. Every topic is linked from another one (the index excepted). | DoD "every new command has a help topic"; UI-006. | 12, 14, 17 |
| D-68 | **Repository tasks.** `xtask/` is a workspace member (binary, `publish = false`) whose only dependency is `hikasa-session`; alias `cargo xtask` in `.cargo/config.toml`. The architecture test covers it (check 1 scans `xtask/`, check 3 lists it). | docs/16 "moved to M2". | 02 |
| D-69 | **Test runner.** CI runs `cargo nextest run --workspace --profile ci` plus `cargo test --workspace --doc`. `.config/nextest.toml` runs the wall-clock latency tests (names matching `cancel`, `latency`, `budget`, `within_`, `timeout`, `abandoned`) one at a time; the `ci` profile retries them twice (a retried pass is reported as flaky). `bench.yml` is manual (`workflow_dispatch`). | Q12 = a; the M1 macOS/Windows timing failures; under load (load average 30) `a_cancel_interrupts_one_long_line` failed once with every test in parallel and passed five times alone. | 02, 17 |
| D-70 | **One settings-file engine** (T13). Every program-owned settings file (`options`, `state`, `commands`, `theme`, `themes/<name>`, `filters`, `openwith`, `archives`) is loaded and saved by `SettingsFile<T>` (§1.1.2). Writes are **sparse**: a key equal to its default is not written, so a later change of a default reaches every user; unknown keys and sections are kept (Q11), comments and layout are not. | One owner of versioning, migration, preservation and atomic writes. | 03 §5 |
| D-71 | **Revises D-31.** `options.toml` is read completely through the typed model of `docs/15` ("Keys", OPT-001…OPT-022). Unknown keys inside a known section are still ignored silently (logged with `--verbose`) and kept on save; an unknown top-level section still warns `unknown_config_key`. A wrong type or a value out of range is an error for every command **except** the `settings` subcommands, which load leniently (the bad value is reported as warning `invalid_config_value` and replaced by its default in memory) so that `settings set` can repair the file. The M1 `ProgramOptions` (three keys) stays as a view computed from the full model. | The M1 subset grew to the whole page set; repair must stay possible from the CLI. | 03 §5.1, 15 |
| D-72 | **One file per list** (SES-051 wins over the page layout of `docs/15`). Archive type masks live in `archives.toml`, command customizations in `commands.toml`, Open With applications in `openwith.toml`, name-filter presets in `filters.toml`, colours and fonts in `theme.toml` and `themes/<name>.toml`. The sections `archive_types`, `commands` and `open_with` of `options.toml` stay reserved (known, empty; their unknown keys are kept). `[appearance]` of `options.toml` holds only `theme_mode`. | `docs/15` lists pages, `docs/03` lists files; each datum needs exactly one home. | 03, 15 |
| D-73 | **Settings keys** (A15). `hikasa settings` addresses every value as `<file>.<path>`: `<file>` is `options`, `state`, `commands`, `theme`, `filters`, `openwith`, `archives` or `themes.<scheme>`; `<path>` is the TOML key path, segments separated by `.`, a segment that is not a bare key written in double quotes, an array index as a decimal segment (`openwith.app.0.command`). A key that names a table addresses the whole subtree (get/list); `set` takes a leaf or a whole array. | One grammar for every file; the CLI is complete before the GUI dialogs (W4) use the same Service calls. | 12 |
| D-74 | **Change detection** (Q4 = a). A `ChangeStamp` is `(length, modification time in ns or none, CRC-32 of the bytes)`; a missing file has the stamp `Absent`. `ConfigWatch::poll` re-reads the stamps of the files it tracks and reports `Changed`, `Created` or `Removed`; it holds no timer (the adapters call it on window focus and every 2 s, D-62). A save first compares the stamp on disk with the stamp of the load: a difference is `SessionError::Conflict` unless the caller passes `Overwrite::Force`. | Two writers (CLI and GUI) without `notify`; the CRC catches same-size same-second edits. | 03 §5 |
| D-75 | **Revises D-21.** The `format` write commands (`new`, `edit`, `delete`, `import`, `enable`, `disable`, `move`, `restore`) write only the files they touch: `formats/<id>.toml` and/or `formats/order.toml`. They never materialize the other built-ins. A built-in cannot be deleted (it can be disabled; `restore` removes a user file that overrides it). | FF-001 operations available headless; D-21's intent (no surprise files) kept. | 10 §1 |
| D-76 | **Open With expansion is pure** (OPT-012). The command line is split into words first (whitespace separates words; `"…"` groups, `\"` is a quote inside a group), then variables are expanded inside each word, so a substituted path is always exactly one argument whatever it contains. `%%` is a literal `%`; an unknown variable, a `%…2` variable without a second item and a dangling `%` are errors. | No shell, no quoting bugs, testable without processes. | 15 |
| D-77 | **Restore defaults** (CLI part of SES-062 for single files): `hikasa settings restore-defaults` deletes the chosen settings files (the next load sees defaults). It is a T12 plan: `--dry-run` prints it; without `--dry-run` it needs `--yes`, else exit 100 (`invalid_input`). Sessions, workspaces and formats are not touched (formats: `format restore`; sessions and the wizard: W2). | T12; destructive commands never run by accident in scripts. | 03 §6, 12 |
| D-78 | **Themes** (UI-020…023). `theme.toml` picks the scheme for light and for dark mode (`light_scheme`, `dark_scheme`) and holds the four fonts; a scheme is `themes/<name>.toml`. The built-in schemes `light` and `dark` are embedded; a file `themes/light.toml` or `themes/dark.toml` overrides elements of the built-in (sparse); any other scheme names a `base` (`light` or `dark`) whose values fill the elements it does not set. Colours are `#rrggbb` or `#rrggbbaa`, written in lower case. Syntax colours are keyed by grammar element name (lower case); an element without an entry uses the eight-colour `fallback` cycle by element id, as in M1. | Editable built-ins without materializing them; every element has a value. | 14 §2 |
| D-79 | **Advisory exceptions.** `deny.toml` ignores the "unmaintained" notices RUSTSEC-2026-0192 (`ttf-parser`), -0370 (`zvariant`), -0371 (`zbus_names`) and -0372 (`zvariant_derive`): GUI transitive dependencies pinned by eframe 0.37 (winit → sctk-adwaita; accesskit_unix → zbus 5). Each entry is removed when an upgrade drops the crate; a security (non-unmaintained) advisory is never ignored this way. | Maintainer, 2026-10-28 (option a); no fix reachable without leaving eframe 0.37. | 02 |
| D-80 | **Session files** (SES-001…006, ARCH-040). A session is one TOML file loaded and saved by the D-70 engine: named sessions `sessions/<folder path>/<name>.toml`, defaults `sessions/_defaults/<type>.toml`, auto-saved `sessions/_autosaved/<id>.toml`. Names and folder names: 1–100 characters, none of `/ \ : * ? " < > \|` or control characters, no leading or trailing space or dot, not starting with `_`, not a Windows device name (`CON`, `NUL`, `COM1`, …); unique per folder **case-insensitively**. A session reference is the folder path and name joined by `/` (`work/api-v2`). | Portable names on every platform; `_` keeps the reserved folders apart. | 03 §1, §5.3 |
| D-81 | **Typed parts now, opaque parts later.** `specs` and `rules` are typed for Folder Compare (`hikasa_folder::FolderRules` + text rules for content tests, the same TOML as `--rules-file`), Text Compare and Text Edit (`hikasa_text::TextRules`, format, encodings). Folder Sync, Table, Hex and Picture sessions store `specs` typed and `rules` as an opaque table kept as it is until their engine wave (W6, W12) types it. `view` (layout, view state) and `display` (display filter, SES-010) are opaque tables in every type until the GUI waves type them. Reserved P1 types (`text-merge`, `folder-merge`, `media`, `version`, `registry`, …) are refused with `unsupported`. | The rules of M1 are already serializable; nothing is invented ahead of its wave. | 03 §1 |
| D-82 | **Effective settings of a session** (ARCH-040/041, SES-021). `rules = defaults(type)` overlaid by the session file's `rules` (a session file stores only what differs from the type's defaults at save time, D-70 sparse against the defaults); for a child session (TC-052 `parent` link) the overlay order is `defaults(type)` → parent `[[override]]` with `scope = "type"` → parent `[[override]]` with `scope = "pair"` matching the child's relative paths → the child's own changes. Scopes: `view` (nothing persisted except the view's own session), `defaults` (writes `_defaults/<type>.toml`, rules only, never specs), `pair` and `type` (write an override into the parent session; the parent must be a named or auto-saved session). | One table, testable without the GUI. | 01 §3.3, 03 §2 |
| D-83 | **Auto-save** (SES-002). One auto-saved file per open view: the first save creates `_autosaved/<YYYYMMDD-HHMMSS>-<n>.toml` (local time of the injected clock, `n` the first free counter) and later saves of the same view overwrite it. The file carries `saved_at` (RFC 3339 with offset). Groups are computed from `saved_at` against the injected local date: Today, Yesterday, 2–6 days ago, More than 6 days ago (future dates count as Today). After each auto-save the store keeps the newest `max_auto_saved_sessions` (OPT-019) by `saved_at`, then by name; 0 = no auto-save and every auto-saved file is removed at the next prune. Named sessions are never pruned. Timers stay in the adapters. | Deterministic with an injected clock (T1). | 03 §1 |
| D-84 | **Lock and close prompts** (SES-004, SES-005) are the pure function `close_action(origin, locked, dirty, explicit_save) -> CloseAction` of the table in §1.2.4. A locked session file is never written by auto-save or by close; an explicit Save on a locked session asks "Save changes to locked session?" and writes only after Yes. | One state machine for the GUI (W4) and tests. | 03 §1 |
| D-85 | **Workspaces** (SES-040…042) are `workspaces/<name>.toml` (D-70 engine): windows with geometry and active tab, tabs in order, each tab either a reference to a named session (`session = "work/api"`) or an inline session table (an unsaved or auto-saved view) plus its opaque `view` layout; an optional `shortcut`. Workspace names follow D-80 (no folders). Loading closes nothing in core; the GUI closes its views and restores (tabs load lazily, G12). A reference to a session that no longer exists restores a Home tab and adds warning `session_missing`. | SES-040; lazy restore keeps the first screen fast. | 03 §4 |
| D-86 | **Settings packages** (SES-060…062). A `.hikasa-pkg` is a zip (stored or deflate, no encryption) holding `manifest.toml` and the files at their configuration-relative paths. Categories: `options` (`options.toml`, `filters.toml`, `openwith.toml`, `archives.toml`), `theme` (`theme.toml`, `themes/*.toml`), `commands`, `state`, `sessions` (named sessions and `_defaults`, per item; never `_autosaved`), `formats` (per item, plus `order.toml`), `workspaces` (per item), `system_info` (`system-info.txt`, export only, never imported); `profiles` is reserved (P1). Import is a T12 plan: every entry is validated (manifest match, allowed path pattern of its category, no `..`/absolute/backslash paths, ≤ 10,000 entries, ≤ 16 MiB per entry and 64 MiB in total uncompressed, each file parses with its schema, none newer than this build) **before** anything is written; then items are written one by one with per-item results. Collisions are items whose target exists (they are replaced). `--silent` imports every category and item without deletes. | Safe, previewable import; packages built from the same files the store reads. | 03 §6 |
| D-87 | **Restore factory defaults** (SES-062) is an import plan against the built-ins: per category, the user files of that category are deleted (options, theme, commands, state, filters, openwith, archives files; `_defaults/*.toml`; format files that override or add formats and `order.toml`); named sessions and workspaces are deleted only with the explicit `--delete-sessions` / `--delete-workspaces` choices (the wizard's "delete all existing" options). It extends `settings restore-defaults` (D-77) with `--category`. | SES-062 "reuses the same wizard". | 03 §6 |
| D-88 | **Command-line forms** (CLI-002, revises D-51 additively). One positional argument that is not an existing path is resolved as: a named session (exact reference, then case-insensitive) → `LoadSession`; else a workspace → `LoadWorkspace`; else the M1 error. An existing file whose name ends in `.hikasa-pkg` (case-insensitive) is a settings package: with `--silent` it is imported headless (D-86), without it the CLI asks the GUI to show the import wizard (W4; until then exit 100 `unsupported`). `@file` stays reserved for W8. | CLI-002 order; an existing path always wins over a name. | 12 §1 |
| D-89 | **IPC** (ARCH-091, additive to protocol v1, D-46). `LoadSession{name, wait}` → `ViewOpened{view}` (and `ViewClosed` when waited); `LoadWorkspace{name}` → `Views{views}` (the views it opened). Unknown names → `Error{not_found}`; a type the GUI cannot render yet → `Error{unsupported_request}`. A GUI that answers `unsupported_request` to these requests is reported by the CLI as "the running GUI does not support sessions; restart it" (exit 100). | Forward compatibility per D-46. | 01 §6 |
| D-90 | **Revises D-82 (how rules are written).** A save writes into `[rules]` and `[text_rules]` (of a named session, an auto-saved one and `_defaults/<type>.toml`) every key that differs from the defaults of the type **now**, plus **every key the file already held**, with its value now: an explicit choice stays written even when it equals the defaults (a later change of the defaults does not change it); a key the file did not hold is written only when it differs. `session edit --unset <key>` removes a key, so the session follows the defaults again. Unknown rule keys, nested ones included, are kept (D-70). The optional folder rules `criteria.timestamps.offset`, `filters.other.size` and `filters.other.date` are written whole, or `false` for "off" where the defaults set them; `false` (and JSON `null`) reads as off, in session files and in `--rules-file`; `true` is refused. Removing an entry of a map that the defaults hold (an `importance.elements` entry) is not representable in M2: deferred to W4 (the GUI's session settings). | Review of W2: an explicit choice must survive a change of the defaults, and "off" must be writable; maintainer, 2026-10-29 (the map entries). | 03 §1, 04 §3 |
| D-91 | **Locked sessions and bulk commands** (SES-004). `settings import` does not replace a locked named session, nor delete one with `--delete-existing sessions`; `settings restore-defaults --delete-sessions` does not delete one. Each is planned as `skip_locked` (never executed, no result, not counted in the summary) with warning `session_locked`; `--force` on `settings import` / `restore-defaults` includes them. `session delete` and `session rename` of a locked session are `invalid_input` unless `--force`, and so are `session rmdir --recursive` and a folder rename (the Home tree) whose subtree holds locked sessions (the refusal names their count; the GUI's prompt names them, confirming is the force); auto-save never writes a locked file. The defaults files are never locked. | Maintainer, 2026-10-29: a lock protects a session from every command that would change it by accident. | 03 §1, §6, 12 |
| D-92 | **Mutating VFS = single-item primitives + one generic layer** (VFS-001, ARCH-031). The `Vfs` trait gains `create_dir`, `remove` (file, link or empty folder), `rename` (one back-end), `set_times`, `set_attrs`, `create_symlink`, `move_to_trash` and the optional `copy_file_fast`, each with a default body returning `OperationUnsupported` and each behind a capability bit (seven new bits, `SET_TIMES` now used). Recursion, cross-back-end copy, move, plans and per-item results live once in `hikasa_vfs::ops` over the primitives. `MemoryVfs` implements every primitive with fault hooks (§1.3.8). | One owner of the algorithms (T11); `MemoryVfs` faults test them for every back-end; W5 archives only implement primitives. | 01 §3.2, 11 §1 |
| D-93 | **Errors of mutations.** New `ErrorKind::NotEmpty` (`not_empty`); `VfsError::{NotEmpty, CrossDevice, OperationUnsupported, InvalidTarget, Unavailable}` with the kinds of §1.3.2 (`CrossDevice`, `OperationUnsupported`, `Unavailable` → `unsupported`; `InvalidTarget` → `invalid_input`); seven new `VfsOp` names. Exit codes unchanged: any `vfs` error or failed item is 100. | "Folder not empty" is a common, user-visible outcome that deserves its own kind; the rest maps onto existing kinds. | 11, 12 §2 |
| D-94 | **Writes never follow the final link** (plan §5.1 "never follow a link being deleted"). `remove`, `rename`, `move_to_trash`, `set_times` act on a link itself; `set_attrs` on a link is unsupported; a copy or move onto a link replaces the link (`WriteOptions::replace_link`), never the file it points to; trees are never descended or written through links; links are copied **as links** (`LinkPolicy::Copy`, or `Skip`), root arguments are followed only with `--follow-symlinks`. `open_write` keeps the M1 follow-the-link save semantics by default. | A copied tree stays faithful, never pulls data from outside the tree and cannot loop; deleting through a link is the classic data-loss bug. | 11 §1 |
| D-95 | **Generic copy** (ARCH-031). Per file: `open_read` → `open_write` to a temporary sibling → `commit` (atomic rename); any read/write/cancel fault drops the writer, so the target is untouched. Local → local copies files ≤ 64 MiB in one `std::io::copy` call into the temporary sibling's open handle (`copy_file_range` on Linux; larger files stream, to keep cancellation ≤ 100 ms). Metadata after content, in the order times, then attributes; a folder's metadata after its subtree; owner never. A missing capability skips the step with one `capability_missing` warning per operation; a failing supported step fails the item and keeps the content. | ARCH-031 text; a read-only result must not block the time change; the OS copy is not cancellable. | 01 §3.2, 11 §1 |
| D-96 | **Overwrite policy** `fail` (default) / `skip` / `always` / `newer` (strictly newer `mtime`; unknown → not newer). A link at the target counts as a file; a folder is never deleted to make room for a file; folders merge. Special files (FIFO, socket, device) are skipped, never opened. At execution an item planned without a target that finds one fails ("changed since the plan was made"). | Safe default for a headless tool; execution never does more than the plan showed (T12). | 11, 12 |
| D-97 | **Move.** Same back-end, rename capability, target absent → one `rename` (atomic, O(1)); known different devices, another back-end or an existing target folder → copy + remove per item (folders merged, a source folder removed only when empty). A `rename` failing with `CrossDevice` at execution is expanded in place. When removing the source fails, the fresh copy is removed again unless it replaced an existing target. | Rename where possible; no duplicate and no loss otherwise; matches the W6 acceptance "copy + delete with rollback". | 11 |
| D-98 | **Remove and trash** (FC-073 primitive). `rm` removes files, links and empty folders; non-empty folders need `--recursive` (also with `--trash`); recursion is post-order and keeps a folder whose child failed (`children_failed`). Trash is `LocalVfs` only, through `trash` 5 (`chrono` for the freedesktop `DeletionDate`), one call per root item, links trashed as links on the same file system (a link on another file system than the home trash is refused on the freedesktop platforms, §1.3.12), and **never falls back to a permanent delete**. A file-system root is never removed or moved. `trash` reads the home/data folders and the clock inside the call (and `local/trash.rs` reads the same two variables for the link check): accepted as an OS side effect (not a result input, T1), confined to `local/trash.rs`; tests isolate it with `XDG_DATA_HOME`/`HOME` in a subprocess. | Recoverable deletes must be recoverable or fail; T1's purpose (deterministic results) is kept. | 11 §1, 02 |
| D-99 | **Times and attributes** (FC-076 primitive). `set_times` sets the modification time and, where `SET_CREATED` (Windows, macOS), the creation time; the access time is never changed. `set_attrs` sets Unix permission bits (`std`) or DOS bits; on Windows only `R` is settable (`H`, `S`, `A` need Win32 calls, i.e. `unsafe`, which is forbidden); owner and group are never changed. `touch` defaults to the injected clock's now. | std-only, no `unsafe`; FC-075 (attributes dialog) is P1. | 11 §1, 04 §7 |
| D-100 | **Clipboard back-end** (TC-002, TC-065 data). A pure `ClipboardProvider` trait in `hikasa_session::clipboard`; `host::SystemClipboard` (adapter-only) reads through `arboard` 3 (text, else image, else empty). The service reads it lazily on the first `clipboard://` spec and once per service; `Service::read_clipboard` re-reads (GUI). Text → UTF-8 blob without a name; image → an in-house stored-deflate PNG named `clipboard.png`. No clipboard (headless, CI) → `unsupported`, empty → `not_found`, both exit 100. Tests use `FixedClipboard` or `HIKASA_CLIPBOARD_FILE`, never the real clipboard. | Q9 (`arboard` approved for the CLI too); the VFS stays free of OS clipboard calls; no image encoder dependency before W12. | 10, 11 §1, 12 §4 |
| D-101 | **Revises D-49.** `gui open clipboard://` and `--gui` with `clipboard://` are allowed: the CLI sends `clipboard://` verbatim in `OpenView.specs`; the GUI reads **its own** clipboard (`Service::read_clipboard`) when it opens the view. `-` keeps the M1 spool; rules files may still not be `-` or `clipboard://`; sessions still never store either (D-80). | `arboard` approved (Q9); the GUI runs on the user's desktop while the CLI may be headless. | 12 §1.1, 01 §5.1 |
| D-102 | **`hikasa vfs cp\|mv\|rm\|mkdir\|touch`** (§1.3.7). Target naming as `cp`/`mv` (into an existing folder, else the new name; several sources need a folder), owned by core (`ops::resolve_targets`). `rm` needs `--yes` unless `--dry-run` (as D-77); the other four need no confirmation because they replace nothing without an explicit `--overwrite always\|newer`. One schema `hikasa.vfs_op.v1`; item failures give `ok: true`, exit 100 (as W2 import); human output one line per item plus an ARCH-101 summary. | CLI-first (T10) before W6/W9 use the same primitives; destructive commands never run by accident. | 12 §1, §4 |
| D-103 | **Plans of VFS operations** (T12, minimal). Every write command builds the complete serializable `ops::Plan` (whole source trees listed, items in execution order) before the first write; `--dry-run` prints that plan; execution re-checks each target and never exceeds the plan; results are per item (`planned`, `done`, `skipped`, `failed`, `not_run`); nothing runs below a folder item that failed or was skipped, or below a folder that changed since planning (`not_run`, §1.3.12). W6's `OpPlan` wraps these items (sides, conflicts, backups, confirmation). | The plan is the unit W6 extends; a finite plan also makes "copy a folder into itself" harmless. | 12 §4 |
| D-104 | **W4 in three sequential parts** (plan §2 "GUI waves may split into a/b"). W4a *Shell* (windows and tabs, commands, theme application, icons, status bar, jobs, message and log panels, notifications and polling, dialog framework, OS drag and drop), then W4b *Sessions* (sessions of views, close prompts, auto-save, Session Settings dialog shell, Home view, workspaces, the GUI command line), then W4c *Settings dialogs* (Options dialog with the toolbar/shortcut/menu editor and the theme editor, Export/Import/Restore wizards, IPC `ImportPackage`). One implementer per part; a part starts when the previous one is merged on `feat/m2`; each part ends with the tree green (its acceptance subset of §6.4 and the full checks). The whole-wave gates (§1.4.17) are re-measured once at the end of W4c. | Size L and the maintainer's "few agents at a time"; each part is reviewable alone and leaves a working window. | 16 |
| D-105 | **The GUI calls of W4** (T11, ARCH-L3). The GUI reaches files only through `Service` calls; it may call pure core functions and types directly (`close_action`, `is_dirty`, `Shortcut::parse`, `Scheme::resolve`, `T::validate`, `CommandOverrides` helpers, `LogBook`). Every `Service` call that reads or writes a file runs off the UI thread (`Service::spawn`, or the watch thread of D-117); the UI thread only calls in-memory accessors. W4 adds **no `hikasa` subcommand**: each new `Service` call is either a typed form of an operation an existing subcommand already performs (same engine function, same files written — a test per call compares the files with the CLI's) or GUI-only state (`state.toml` layout, the log book). | CLI-first (plan §0.1) without duplicate commands; the UI thread never blocks (M1 §1.10). | 01 §3.6 |
| D-106 | **Window model** (UI-001, SES-090). Each top-level window is an egui viewport shown with `show_viewport_immediate` from the root viewport's frame; one `AppCore` owns every window (`Vec<WindowModel>`, toolkit-free), each window owns its tabs, and a view (its `ViewId`, jobs, IPC waiter) lives in exactly one tab. `windows[0]` is always drawn by eframe's root viewport: closing it while other windows are open moves the next window's model into the root viewport (its geometry is applied) and closes that window's viewport, so the process never ends while a window is open. Opens from IPC, the command line and Home go to the window focused last; `[tabs] open_sessions_in = "window"` and `open_child_sessions_in = "window"` open a new window instead (OPT-004). The platform's close of the root viewport is cancelled whenever the program does not quit (another window remains, or a prompt asks), so eframe keeps running after the root swap. A window's viewport builder carries its geometry only when its viewport opens; later frames change only its title (the platform's reports never feed back). | One owner without locks; eframe ends the process when the root viewport closes; immediate viewports repaint on demand like M1. | 14 §1, 03 §9 |
| D-107 | **Tab moves** (SES-090). New global catalogue commands (context menu of a tab, no default shortcut): `TabDuplicate`, `TabMoveToNewWindow`, `TabMoveToWindow` (a submenu of the other windows), `TabCloseOthers`. Dragging a tab inside its tab bar reorders it; releasing it over another window's tab bar moves it there **when the platform reports window positions**; releasing it outside every window opens it in a new window at that position under the same condition. Where positions are unknown (Wayland, A17) a release outside the source tab bar does nothing and the two Move commands remain the way. A window whose last tab moves away closes. Duplicate opens a new view of the same session (its own comparison, no shared model) right of the source tab. | Cross-window pointer delivery is platform-dependent (plan §5.1); the commands always work and are testable without a window. | 03 §9 |
| D-108 | **Layout memory** (UI-001). `state.toml` gains `[layout.<view type>]` (an opaque table per view type, keys owned by the views: W4 defines `toolbar`, `log_panel`, `log_height`, `font_size`, `thumbnail`; W9/W11/W13 add theirs) next to `[window]`, which holds the geometry of the last window closed. The schema stays version 1 (an additive known section). Precedence when a view opens: the session's `[view]` keys (D-81) → `[layout.<view type>]` → built-in defaults. The GUI writes `state.toml` only through `Service::state_update`, which loads leniently, applies the change, saves with `Overwrite::IfUnchanged` and on `Conflict` reloads and applies once more. Sizes are restored on every platform, positions where the platform allows (A17). | Per-type layout survives restarts without touching sessions; the closure keeps concurrent CLI edits of `state.toml`. | 03 §5.2, 14 §1 |
| D-109 | **Effective command table in core; GUI presentation only** (UI-003, refines D-65). `hikasa_session::commands::effective(view, &CommandOverrides) -> ViewCommands` merges the catalogue entries of the view's sections (`view_sections`) with `commands.toml`: shortcuts (override replaces the default; `[]` removes), menu and toolbar visibility, the four per-view flags. A catalogue entry gains the optional field `toolbar` (bool, default `false`: on the default toolbar); `docs/reference/commands.md` keeps its layout. Menus, toolbars, context menus, shortcuts and the Shortcuts page show **`implemented` entries only**. The GUI's own table keeps per id only presentation: icon, toolbar label, radio group, availability predicate and context-menu area; captions, descriptions (hints), menus and shortcuts come from core. | One source of captions and shortcuts for the GUI, `commands-doc` and `settings`; overrides apply in one place. | 01 §3.6, 14 §1 |
| D-110 | **Menu assembly** (UI-002, UI-004, UI-005). Menu bar: Session, File, Edit, Actions, Search, View, Tools, Help; a menu without a visible entry is hidden. Inside a menu, entries are grouped by catalogue section in `view_sections` order (own section first, `global` last) with a separator between groups, catalogue order inside a group; the Session menu puts the `global` entries before `CloseTab` first, then the view's groups, then `CloseTab` and `Exit`. An entry is visible when it is `implemented`, has a caption and a menu, and its effective menu flag (or `show_all_in_menu`) is set. `MacWindow` entries are keyboard-only. The pseudo-menus `DropDown`, `Favorites`, `Toggles` are the display-filter presentation chosen by `display_filter_mode` (toolbar control; View ▸ Display Filter submenu). Entries with an empty `menu` appear only in context menus and on the toolbar. Context menus are lists of command ids per area (tab, toolbar, sessions tree in W4; panes in W9/W11/W13). | Deterministic menus derived from data, testable as text. | 14 §1 |
| D-111 | **Shortcut dispatch** (UI-003, UI-006). A key press becomes a core `Shortcut` (on macOS a catalogue `Ctrl+` is the Command key). Candidates are the visible, available entries of the active view whose effective shortcuts contain it. (1) An entry whose shortcut has a context (`Enter (in a path field)`), whose `menu` is empty, or whose shortcut is focus-bound (no Ctrl/Alt/Cmd and a key among letters, digits, punctuation, Enter, Tab, Esc, Space, BkSp, Del, Ins, Home, End, PgUp, PgDn, arrows) is taken only by the focus area it belongs to. (2) Among the rest the first entry in `view_sections` order, then catalogue order, wins. (3) A workspace shortcut (SES-040) applies only when no command took the key. While a dialog is open in a window, that window's commands take no key. egui does not tell the keypad apart: a Ctrl press of `=`/`+`, `-` or `0` is tried as typed (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`) and then as the keypad key (`Ctrl+Num +`, …), the first binding wins. The focus area is the focused widget's (a Home path field, the folder tree, the text panes): its area is tried first, then the window root; while a dialog is open the view under it takes no key. | Extends the M1 focus rule to overrides and context-only commands; the catalogue's deliberate duplicates (Copy/Take) are resolved by focus. | 14 §1 |
| D-112 | **Dialog framework** (SES-092 framework, UI-006). Dialogs are egui windows inside their native window, modal to it (a dimmed backdrop takes the window's input), one stack per window; never extra viewports. A dialog is a toolkit-free model (`DialogModel`: title, help topic, buttons, `on_button`, `on_key`) tested without a window. Esc = Cancel (or Close), Enter = the default button, F1 = the dialog's help topic; the OS close button = Esc, so a dialog that asks before Esc asks before it too (`DialogModel::on_close`, W4 leftovers in §1.4.20). The title bar moves a dialog. `Prompt` (message, buttons, default, optional "Do not ask again" bound to an option key) serves D-84 questions, the multiple-close warnings and conflicts; W6 confirmations, W8's status window and W11's per-file prompts (SES-092) reuse it. | Cheaper than viewports (memory gate), works on Wayland, testable. | 14 §4 |
| D-113 | **Theme application** (UI-020…022, FC-031). The window's colours come only from the resolved `Scheme` of `ThemeSelection::scheme_for(theme_mode, os_dark)` (`os_dark` = egui's system theme, re-evaluated when it changes); egui's chrome visuals start from the scheme's base (light or dark) with the selection colour of `[folder] selection` unless `use_system_selection`. `ui/theme.rs` maps scheme elements to the painters' colours through one data table. Fonts: the four `theme.toml` sizes apply (editor: text panes, folder: tree, listing and hex: their W11/W13 views); `family = ""` is the built-in font; a non-empty family is looked up and applied by D-160 (W4: built-in font and one notice, no font discovery), and a family not found keeps the built-in font with one warning notice per family. egui's own theme is fixed to the scheme's base (`Context::set_theme`) and both of its styles carry the palette, so egui never switches styles by itself when the OS theme changes; under `follow_os` the core picks the scheme. | Every colour configurable through one model; fonts by D-160. | 14 §2 |
| D-114 | **In-house vector icons** (UI-010). Icons are constant lists of drawing operations (lines, polylines, polygons, circles, rounded rectangles) on a 16 × 16 design grid, painted with egui's painter at the window's current pixels-per-point in a colour from the palette, so they are sharp at 100/125/150/200 % and any other scale, in light and dark schemes. No image files, no icon font, no dependency. The inventory is §1.4.5; every toolbar entry of the presentation table and every session type has an icon (test). Original drawings only (`docs/17` §5). | Scale-free and small; nothing copied. | 14 §1 |
| D-115 | **Status bar, jobs, message panel** (UI-030). Each window has a status bar: left the active view's context text, right its state items (comparison state, then encodings and line endings for file views; zoom and insert mode arrive with their views), and a job indicator that aggregates the running jobs of every view of the window (title, phase, fraction when known, count); clicking it lists the jobs, each with Cancel; `Stop` cancels the active view's jobs. Each view has a message panel (one line, kind info/warning/error, a close button, optional "Do not show again" bound to an option key such as `[next_difference] show_message_panel`). The M1 window-wide message is replaced by the view's panel; About becomes a dialog. | UI-030 as specified; progress per window, cancel per job. | 14 §3 |
| D-116 | **Log book** (FC-100 panel). `hikasa_session::log::LogBook` is a pure bounded list of entries (`time`, `kind` summary/detail/warning/error/debug, `source`, `text`); OPT-018 filters at insertion (errors always kept), `display_up_to` is the capacity (oldest dropped). One book per GUI process, shared by every window and shown by the folder views' Log panel (`ShowLog`; later views add it). With `save_to_file` set, new lines are appended by `Service::log_append` from the watch thread. Sources in W4: finished jobs (summary), warnings and errors of every `Service` result the GUI receives, notifications, settings reloads, workspace and package results, and `hikasa::*` tracing events at debug level when `debug_messages` is on. Appends run one at a time (lines pushed meanwhile go with the next append, in order). A failed append stops saving, with one warning notice, until the settings change; the failure never goes through tracing (no loop through the book). The tracing layer posts debug events only while `debug_messages` is on (an atomic flag the core keeps current). | W6 file operations and W8 scripts append to the same book. | 04 FC-100, 15 |
| D-117 | **Polling and notifications** (UI-031, SES-053, D-62). A GUI thread `hikasa-watch` (an adapter timer, D-83) polls every 2 s and at once when any window gains focus: `ConfigWatch` over every settings file, `themes/` and `formats/`, then the stamps of the inputs of open file views, then the stamp of the session store (D-124). Results are posted to the UI thread. Reactions are the table of §1.4.7; paths the GUI wrote itself are acknowledged first and raise nothing, the first-run format files included (they are written on the watch thread, between two polls). Notifications are non-modal, in the window concerned (settings: the window focused last): at most three visible, newest on top; info ones hide after 8 s, warnings, errors and those with actions stay until dismissed; equal keys replace each other; each is also a log entry. Reloads run through the window's settings queue with its own writes, one at a time; each queued job has a sequence number and only the snapshot of the newest job is applied (an older one only when every later job failed), so an edit of the window is never reverted in memory. | No `notify` (Q4); the UI never waits for the disk. | 03 §5.2, 14 §3 |
| D-118 | **Changes of input files** (OPT-015). The inputs of open file views (W4: Text Compare) are watched by `InputStamp` = (length, modification time) from the VFS `stat`, never by reading content (inputs may be gigabytes); `-`, `clipboard://` and other blobs are not watched; folder views' bases are not watched (FC-082 refresh is P1). On a change: with `automatically_reload` and no unsaved edits (always none before W11) the view reloads and an info notice says so; otherwise a warning notice offers Reload. A removed input gives a warning notice. `check_for_files_changed_on_disk = false` disables the watch. The watch counts the views of each input (one `Track` per view and input): an input stays watched until its last view untracked it, and a second `Track` keeps the existing stamp (a change not polled yet is still reported). | Cheap polling; the edit-aware prompt of W11 builds on it. | 15 |
| D-119 | **Reloading settings in a running service.** `Service::reload_settings()` re-reads `options.toml` (leniently, warnings returned) and the format registry and swaps them atomically; later calls and jobs use the new values, running jobs keep theirs. `Service::options()` returns `Arc<ProgramOptions>` and `Service::registry()` an `Arc<FormatRegistry>` snapshot (signature change, callers adapt). Options, their ignored keys and the registry are one `LoadedSettings` behind one lock (`Service::loaded_settings()`, a consistent snapshot); a reload holds a mutex from its first read to its swap, so concurrent reloads never finish out of order. `Service::ignored_option_keys()` returns a `Vec<String>` refreshed by each reload (signature change). The GUI's own writes of `options.toml` reload in their job (no outside-change notice). | The GUI is long-lived; SES-053 "reload" must reach comparisons started later. | 01 §3.6 |
| D-120 | **Sessions of views** (SES-001…005, D-84). Every comparison view owns a `ViewSession`: origin (`Unnamed`, `AutoSaved(id)`, `Named(ref)`), the session as loaded, locked, `explicit_save`, its auto-save slot. The current session is built by core (`Service::session_for_view`); `dirty` is `is_dirty(loaded, current)`; closing a view runs `close_action` and acts on its `CloseAction`. Closes that cannot ask — IPC `CloseView`, IPC `Shutdown`, the end of a desktop session — answer **No** to both questions (nothing is written for a named session; unnamed and auto-saved ones are auto-saved as usual); the D-23 outcome of a waited view is unchanged. Save on an unnamed or auto-saved view opens Save Session As; after a save the origin becomes `Named` and the auto-saved slot of the view is deleted. | One state machine (D-84) for every close path; non-interactive callers never block on a prompt. | 03 §1 |
| D-121 | **Auto-save timer** (SES-002, D-83). In the GUI adapter: a view of origin `Unnamed` or `AutoSaved` is auto-saved when it closes (D-84), at program exit, and every 300 s while its current session (`view` and `display` included) differs from the last auto-saved one. Each save is `Service::session_autosave` (the injected clock; prunes to OPT-019). A named view is never auto-saved. The period is a constant (no option exists). | SES-002 "periodically while open" with a deterministic, testable rule. | 03 §1 |
| D-122 | **Session Settings dialog shell** (SES-020, SES-021). A dialog with a tab registry per session type: W4 provides the **Specs** tab (left/right inputs with MRU lists and drop, description, for file types the format and the two encodings) for the types the GUI can show, and the footer's scope selector (*this view only*, *update session defaults*, and for a child session with a named or auto-saved folder parent *these files within parent* and *all files within parent*). The default scope is *this view only*. Specs never go to the defaults (D-82). Rule changes are applied with `Service::session_apply_scope`; the rule tabs are added by W9, W11 and W13 into the same registry, so in W4 only test-registered tabs change rules. `Rules` (SES-022) stays `planned` until the first rule tab exists. | Framework now, tabs with their views; scope semantics already in core (D-82). | 03 §2 |
| D-123 | **Removing an inherited map entry** (W2 note, D-90 deferral). A session or override removes an entry of a rule map that the defaults hold (`importance.elements.<name>`) by writing the entry as `false`; the effective rules drop it; `session edit --set rules.importance.elements.<name>=false` is the CLI form. Implemented in **W11** with the importance tab, the first editor that needs it (moved from W4). | Same convention as D-90's optional rules; no GUI in W4 can remove such an entry. | 03 §1 |
| D-124 | **Home view** (SES-030, SES-031, SES-093). Type buttons (one per session type the GUI can open; drop targets), a search box (`Find`, Ctrl+F), the sessions tree and the management panel (`ShowManage`, `MaximizeManage`). Tree: *New* (one entry per type: its defaults), named sessions in folders (folders first, then favorites, then the rest, each case-insensitively sorted), *Auto-saved* in the four D-83 groups, newest first. Search: case-insensitive substring of the name, folder path or description; matches are shown with their ancestors. The M1 path form becomes the management panel's Specs editor; `HomeCompare` opens the selection. The tree's context-menu commands (SES-031) are new catalogue entries of the `home` section (§1.4.11). The tree refreshes when Home becomes visible, when a window gains focus, after the GUI's own writes and when the watch poll sees the session store change (D-117). `SessionHome` (Alt+Home) replaces the view by Home after its close action; `SessionsCombo` lists the named sessions of the view's type and opens the chosen one in this tab. | SES-030/031 with the store of W2; `sessions/` is polled by stamp only (paths, lengths, times; *revised in the W4 leftovers*: a CLI `session delete` must show without a focus change). | 03 §3 |
| D-125 | **Workspaces in the GUI** (SES-040…042). Save Workspace As captures every window (root first, then in opening order): geometry, active tab, and per tab a reference for `Named` views, an inline session (`Service::inline_table`) for the others, the pending session of a not yet restored tab, and the view layout; Home tabs are not saved (D-85 has no Home tab; a window without saved tabs restores as one Home tab). Load Workspace runs every open view's close action first (a Cancel aborts the load), then restores the windows lazily (D-126). A workspace `shortcut` is dispatched by D-111 rule 3; Manage Workspaces renames, deletes, edits the shortcut (conflicts with global commands and other workspaces are shown) and loads. Startup: without view arguments and with `[startup] load_workspace` set, that workspace is loaded (missing: a warning notice and Home); exit: with `on_exit_save_workspace_as` set it is saved after the close warnings and before the views close. IPC `LoadWorkspace` restores windows the same way. | SES-040/041 on the W2 files. | 03 §4 |
| D-126 | **Lazy restore** (G12). A restored tab is *pending* (its session or reference, title, layout) until it is first activated; only the active tab of each window starts its comparison. A pending tab is registered with the IPC view list (`gui list`) and closes with `close_unopened` semantics. A pending tab's caption is the session reference, else its description, else the input names. | First screen < 1 s whatever the workspace size; memory per unopened tab is a session value. | 03 §4 |
| D-127 | **GUI command line and `ImportPackage`** (CLI-002, SES-061; revises D-88's wizard clause, additive to protocol v1 per D-46/D-89). `hikasa-gui <one argument>` uses `Service::classify_single_arg` like `gui open`: a session or workspace opens (or is forwarded as `LoadSession`/`LoadWorkspace` to a running GUI), a `.hikasa-pkg` without `--silent` shows the Import wizard. New request `ImportPackage{path}` (absolute path) → `Ok` once the wizard is shown in the window focused last (an open wizard takes the path on step 1, is replaced after its result, asks in between, and raises a notice while a job runs); a GUI that answers `unsupported_request` is reported as for D-89. `hikasa gui open <file.hikasa-pkg>` without `--silent` sends it (exit 0) instead of exiting 100 `unsupported`. | Completes D-88; old GUIs fail clearly. | 12 §1, 01 §5.1 |
| D-128 | **Options dialog** (OPT, docs/15). Searchable page tree on the left (pages and sub-pages of §1.4.13), page on the right, Apply / OK / Cancel. Each P0 key of `key_infos(FileId::Options)` appears on exactly one page; P1/P2 keys are not shown (they stay stored and validated). Widgets follow `ValueKind`; values are validated live with the schema's `validate`, and Apply is disabled while a problem remains. The dialog edits a typed working copy (`SettingsSnapshot`) taken when it opens; Apply writes the changed files with `Service::settings_save` (`IfUnchanged` per file; a file equal to its snapshot is not written); a `Conflict` asks *Overwrite* / *Reload this file (discard my edits)* / *Cancel*; a file on disk that cannot be read (a newer `schema_version`, broken TOML) is never overwritten: `Unreadable`, the page says why, and only *Reload* and *Cancel* are offered; OK = Apply, then close when every file of the run succeeded (a file that failed keeps the dialog open, also after a later Overwrite of a conflict); Cancel discards unapplied edits and ends the theme preview. Only the files `changes()` would write are checked: a file left alone never blocks. Esc leaves an open popup first and asks when edits are unapplied; the OS close button asks the same. Search matches page titles, labels, key paths and OPT ids, case-insensitively; a page the search does not match gives way to the first one that it does (no match: no page). | One dialog over the W1 models; the CLI's sparse, preserving writes (T13). | 15 |
| D-129 | **Toolbars, Shortcuts, Menus page** (UI-003, OPT-011). View selector (the view types the GUI shows), search over id, caption, description and shortcut, and a list of the view's `implemented` commands with Menu and Toolbar check boxes (Toolbar only for commands with an icon) and their shortcuts: Add (capture a key), Remove (the selected shortcut; removing all writes `shortcuts = []`; a list equal to the default is no override), Default (drops the shortcuts' override; off when there is none). Conflicts (`CommandOverrides::conflicts`) are computed live on the working copy and shown on both rows; they never block Apply. Also the view's display-filter mode, large buttons, show all in menu and show all on toolbar. Edits use pure `CommandOverrides` helpers; overrides of `planned` commands in the file are kept untouched, like views and commands this build does not know. | UI-003 editor over the W1 model; conflicts are warnings (W1). | 14 §1, 15 |
| D-130 | **Theme editor with live preview** (UI-020…023). The Appearance pages edit working copies of `theme.toml` and the scheme files: mode, the light and the dark scheme, every element of `[folder]`, `[file]` (with syntax elements and the fallback cycle) and `[picture]`, the flags, the four fonts; per element *Reset* (back to the base value: removed from the file); New (name and base), Clone, Delete (not for built-ins; *Reset all* on a built-in deletes its override file, unknown keys included; the last element reset one by one deletes it only when it holds no unknown key). Every change is applied at once to every window (the preview is always on); Cancel restores the applied theme; Apply writes `theme.toml` and the changed scheme files (sparse, D-78). Default text below 4.5:1 contrast on a background is a warning, not an error. | UI-020 "live Preview in the open view" without a separate preview mode. | 14 §2 |
| D-131 | **Export, Import and Restore wizards** (SES-060…062). Step dialogs (Back / Next / Finish / Cancel) over the W2 calls: Export = categories and items → target file (default `~/hikasa-settings-<YYYY-MM-DD>.hikasa-pkg`, injected clock) → `settings_export`; Import = package → the dry-run plan of every category → categories, items (collisions in red, locked sessions kept unless "include locked sessions"), delete-existing choices → summary (dry run of the exact choices) → `settings_import`; Restore = categories and the two delete choices → the dry-run plan → `settings_restore_categories`. A typed path must be absolute (`package::package_path`); nothing is expanded. Finish re-plans in core (T12); when the executed plan differs from the summary shown, the result page says so. The window and Exit are refused while a dialog's job runs. Written paths are acknowledged, then settings reload as in D-117; open views keep running. | SES-060…062 as wizards over the CLI's calls (T11). | 03 §6 |
| D-132 | **Measuring the W4 gates** (§1.4.17). Scripted bench modes of the binary (`HIKASA_GUI_BENCH=scroll\|folder\|options\|two_windows\|workspace`) driven by `gui/hikasa-gui/tests/perf_gate.rs`, release build, **only** under `env -u WAYLAND_DISPLAY xvfb-run -a`; the test fails when the renderer is not a software one (llvmpipe), so the real display is never used. Memory = PSS from `smaps_rollup` (MiB; 400 MB = 381.47 MiB) plus peak RSS, sampled after idle frames (`memory_settled`), the median of three whole runs for `text_scroll`, `two_windows` and `two_windows_late`; under Xvfb the frame-rate item of ARCH-GUI-003 is checked as UI frame work (p95 < 8 ms, max < 16.7 ms), the ≥ 60 fps figure on a GPU display is the maintainer's run (W14). | `docs/17` §3.5; llvmpipe cannot reach 60 fps (docs/02). | 02, 17 |
| D-133 | **No new dependency in W4.** egui/eframe draw everything (icons D-114); fonts were the built-ins (D-113) until D-160 made `fontique` a direct dependency of the GUI, the only exception, and no new package; the Web Resources panel (`ShowWeb`) and website links stay `planned` until `open` (D-63) enters with print (W7/W13). | Plan §0.5; smallest surface. | 02 |
| D-134 | **glibc's mmap threshold** (W4c review). On glibc the GUI replaces itself once at the very start with `MALLOC_MMAP_THRESHOLD_=4194304` set, unless the variable is set already (a failed `exec` just goes on). Buffers of 4 MiB and more go back to the system when freed instead of staying in the arenas' free lists once glibc's dynamic threshold has climbed (up to 32 MiB). The child processes inherit the variable, which is harmless to them. | Measured: 125 MiB of a settled comparison and the bimodal 25 MiB of the W4 gates (docs/02, W4c). `mallopt` is `unsafe` (architecture check 1) and another allocator a dependency (D-133). | 02 |
| D-135 | **W5 in two sequential parts.** W5a *Read* (every format read, resolution, cache, safety, time, CRC, FC-015 scan, read commands, the GUI's rows, dependencies, G6), then W5b *Write* (batches, overlay, rebuild, `create_archive`, the write capabilities). One implementer per part, each ends green; at the end of W5a archives are simply read-only. | Size L; the capability bits make the cut safe, and the risky code (rebuild) gets a review of its own. | 16 |
| D-136 | **`ArchiveVfs` is stacked on a base `Vfs`** and plugs into `VfsRouter` with `with_archives`. It reads containers only through the base, so `MemoryVfs` tests it with faults. Formats are `ArchiveFormat` (the ten types of `archives.toml`, chosen by mask, never by content); a single stream is a one-member archive named after the container without its last extension. Capabilities per format: §1.5.3. Links are reported and never followed. | No special back-end; T11; masks arrive as a value because `hikasa-vfs` cannot name the session. | 01 §3.2, 11 |
| D-137 | **Spec model.** `Spec::parse` is unchanged and never yields `ArchiveMember`. `member: ""` is the archive as a folder, canonical `/x/a.zip/`; members are `/`-separated on every platform; nesting wraps the spec. `Spec::parent` walks out of an archive into the folder that holds it; new `archive_member`, `container`, `outermost_file`, `archive_depth`. | The M1 round trip holds; a root is told from the file by its trailing separator. | 01 §3.1, 11 §2 |
| D-138 | **Resolution** (normative, §1.5.4) is `hikasa_vfs::archive::resolve_spec`: a real item wins; else the deepest existing prefix decides; only a regular file whose name matches a mask opens a way in; `..` never enters. It uses only `Vfs` calls, opens the archive only when a nested name needs it, never fails. `SpecUse::{Any, Folder}`: an archive file stays a file unless a folder is wanted, or the input ends in a separator. Masks ignore case, members never do. `Service::parse_spec` calls it, so every command gets it. | One owner; a real folder `x.zip` stays a folder; `vfs stat a.zip` and `vfs ls a.zip` both do what they say. | 11 §2 |
| D-139 | **Index and reads.** One index per archive and stamp. Zip: the crate's central directory plus an in-house read of the time fields, without touching local headers. Tar family: an in-house block walker over `tar::Header`, because `Entries` buffers extension headers without limit; it hashes every member. Content is never a live decoder: zip members decode to memory (≤ 1 MiB) or a cache file; plain tar members are file ranges; a compressed tar spools the first member read and writes the whole stream to the cache at the second; a stream that does not fit keeps spooling. New `Vfs::prepare`, `Vfs::archive_entries`, `open_read_with`, `VfsSource::with_cancel` keep G4 for long decodes. Hostile input binds an index too: text is counted against `paths_max` as it is read, a folder that only exists through its members counts as an entry and shares a member's text, and the depth is capped (rule 13, §1.5.5). | `ZipFile` borrows its archive, so no owned reader exists in safe Rust; listing and CRC never write to disk; many reads of a `tar.gz` stay linear. | 11 |
| D-140 | **Temp cache.** `ConfigLocation::cache_dir` + `archives`, never the temp folder first (often a RAM disk). `s<n>/` per session, `n` random, files with numeric names only, 0700. A held `File::try_lock` on `s<n>.lock` marks a live session; a later session sweeps folders whose lock it can take, when that file is still at its path, and removes them while it holds the lock. Each file carries a stamp checked at every open. `cache_max` with least-recently-used eviction of files no reader holds. Key = archive key + member; a new stamp drops the files. `cleanup` and `Drop` remove the folder; nothing is created by listing. | A crash cleans itself on every platform without process ids or clocks; no member name can reach a path; concurrent sessions cannot serve each other's bytes. | 03 §5, 11 |
| D-141 | **Member names** (§1.5.7 rules 1–4, 17). Checked on raw bytes before anything else: NUL, absolute, drive letters, `..`, over 4,096 bytes are **refused**: left out of the tree, shown as `<refused-N>` rows with a warning, kept by a rebuild. Zip reads `\` as `/` (legacy tools); tar refuses `\`. File vs. parent, and anything below a link, are refused whatever the order. Duplicates: the last wins, with a warning. Case twins are two members. `LocalVfs::child` on Windows refuses names that are no file names there. | Safe by construction, visible rather than silently dropped, nothing lost on rewrite. | 11 |
| D-142 | **Link, device and special members.** A link member is never followed and never materialized: not in the cache, and a copy out of an archive skips it whatever `LinkPolicy` says (one warning); between archives it stays a link. Hard links resolve to an earlier valid member, else refused. Devices and FIFOs are `Other`. Sparse members are listed, not read, kept by a rebuild. | Revises D-94 rule 6 for archive sources: a link from a hostile archive must not appear on disk. | 11 |
| D-143 | **Limits** (`[limits]` of `archives.toml`, OPT-010): 1,000,000 entries, folders that only exist through members included; 256 MiB of paths and link targets per index; 256 folders of depth; 64 GiB per decoding pass and over overlapping zip members; 8 GiB of cache; ratio 2,000:1 past 64 MiB; nesting 8. Declared and streamed sizes must agree; xz and zstd windows capped at 1 GiB. Errors `ArchiveLimit` (`too_large`) and `ArchiveCorrupt` (`invalid_data`). Listing never needs a declared-size limit: limits bind what is decoded. | Bombs stop at the first of several bounds; a 200 GiB zip or plain tar can still be listed (a zip reads its directory, a plain tar decodes nothing and is read once, with progress and cancellation); a compressed tar is decoded to be listed, so past 64 GiB it needs `stream_max_gib` raised; each limit is a settings key. | 11, 15 |
| D-144 | **Encrypted members** (no AES in M2): name, size and time listed, `crc32: None`, reads → `ArchiveEncrypted` (`unsupported`), one warning, an error row in a content test. No password is asked, tried or guessed. A rebuild copies them raw. | Reported, never guessed; the P1 password prompt can build on it. | 11 |
| D-145 | **Corrupt input never panics.** Every decoder and index pass runs in `catch_unwind`. Tar family and streams keep the members before the damage and add a `<damaged>` row; a zip without a central directory is `ArchiveCorrupt`; a mask match with other content is `ArchiveCorrupt` only when entered, and stays a normal file otherwise; a rebuild refuses a damaged archive. Warnings of the back-end are collected and drained by `Vfs::take_warnings`. | Error rows, not failed commands; a rewrite must not lose a tail it could not read; `Vfs` calls stay free of a warning channel. | 11 |
| D-146 | **Time** (normative, §1.5.8). Zip: NTFS field, then 0x5455, then DOS wall time in the injected zone (`compatible`: gap forward, fold earlier), invalid fields = none. Tar: PAX `mtime`, then the header. Streams: gzip `MTIME` when set, else the container. Written zip members carry DOS time and 0x5455. The zone comes from the injected clock (T1). | Plan §5.1 risk; exact fields first, so most archives do not depend on a zone at all. | 11 |
| D-147 | **CRC.** Zip's central value is `Entry::crc32` and `CRC_PROVIDED`, used **as it is** by `CrcComparator` (no decode, `None` when encrypted); the other formats get it computed by the index pass, `CRC_PROVIDED` off. | FC-025; a second decode is never needed for a CRC test. | 04, 11 §3 |
| D-148 | **Batches** (W5b). `Vfs::begin` and `Vfs::finish(at, Commit\|Abort)`, the W3 hook with an outcome. `ops::execute` batches each archive it touches; primitives only change an overlay every reader sees; outside a batch a change rebuilds at once, so a plain `open_write` cannot be lost. Target roots finish before source roots; a move into an archive removes its sources only after the rebuild. An abort or failed rebuild turns the root's done items into failed ones. | One rebuild per operation; no path loses data when a rebuild fails. | 11 |
| D-149 | **Rebuild protocol** (normative, §1.5.10). `open_write` on the container (temp + atomic rename, Q8), kept members raw: `raw_copy_file` in zip, raw blocks in tar; new or touched members after them, sorted; levels deflate 6, gzip 6, bzip2 9, xz 6, zstd 3; gzip header without time or name, so rebuilds are repeatable. A changed stamp or a damaged archive stops it; a chain of nested archives commits only at the outermost file. A rebuild keeps what it did not touch. A zip with a prefix is refused. | Faults leave the archive untouched by construction; cost is one copy of the archive, documented. | 11 |
| D-150 | **Empty archives** (FC-077), revises §1.3.10: `create_dir` stays literal, because `ops` also uses it for a real folder named `x.zip`. `plan_mkdir` turns a final name with a writable mask into the new `Action::CreateArchive`, run by `Vfs::create_archive`. Single-stream masks and missing ancestors stay folders. | The W3 hook would have broken copying a folder named like an archive. | 04, 11, 12 |
| D-151 | **FC-015.** `handling.archives` = `as_files`, `once_opened` (default), `always`, plus `open_archives` (paths). The scan turns a matching file into a folder pair right after listing, so alignment, filters and flatten need no change; `NodeFlags::ARCHIVE` marks the row; a failed listing is the existing error row; a scan opens at most 5,000,000 entries, reserved from the index before an archive opens. A base may be an archive in every mode. Headless default results equal M1. | FC-015 without a new node kind (D-20 holds); a recursive archive cannot fill the model. | 04 |
| D-152 | **CLI.** No new subcommand, no new schema: `vfs`, `quick`, `text compare`, `folder compare` work on specs; new `--archives files\|opened\|folders`, `--open-archive`. `hikasa.vfs_op.v1` gains the action `create_archive`. Exit codes unchanged. `quick a.zip b.zip` compares the files. New help topic `archives`; no catalogue status changes. | T10: complete before the GUI; a spec is a spec. | 12 |
| D-153 | **GUI.** A zipper icon on `ARCHIVE` rows; `Open` on a closed archive row with `once_opened` adds its path and re-runs the view; `infer_view_kind` makes two archives, or an archive and a folder, a Folder Compare; `Service::shutdown` at exit. No new dialog, command or setting. | Archive sides in the existing views only; W9 and W6 refine the re-run. | 14 |
| D-154 | **Dependencies** (§1.5.15). `zip` 9.2.0 (`deflate`, `deflate64`, `bzip2`, `zstd`, `lzma`, `xz`), `tar` 0.4.46 (no `xattr`), `flate2` 1.1.10 (`zlib-rs`), `bzip2` 0.6.1, `lzma-rust2` **0.23.0** (as `zip`, not the newest 0.24), `zstd` **0.14.3** (as `zip`, not the plan's 0.13; BSD-3-Clause). `deny.toml` gains a crate-scoped exception for `libbz2-rs-sys` (`bzip2-1.0.6`, permissive), pending the maintainer's objection. `zstd-sys` builds C with the `cc` crate: a C compiler per target, nothing else; verified for all packaging targets; `build-linux.sh` pins `CFLAGS` for its target. | One LZMA and one zstd in the tree; plan §5.1's platform risk closed by a real build. | 02 |
| D-156 | **Shadowed duplicates stay** (W5b review; revises rule 3 of §1.5.7, D-141 and A5.16, which dropped them; the maintainer's decision). A shadow shares its winner's fate: winner kept or touched → the shadow stays as raw bytes; renamed or moved inside its archive → it moves with it as raw bytes; moved to another archive, of any writable format → it travels too: each record is decoded into the cache and written right before the member, oldest first, with its time and attributes, so the member still wins; moved to a plain file, which holds one version → it ends with the move, with a warning that says so; removed or replaced → it goes, with a warning that names the count, so an overwritten secret does not survive in an older record. What has no content to take (an encrypted zip record, a tar hard link) stays behind on a move to another archive, with the same warning. `--dry-run` tells the records that would go from the ones that would travel, and learns of those that stay behind only in the run. Shadows are decided before hard links: a link to a kept shadow stays a link, and a winner that is a link written as a file keeps its shadow. A shadow no rewrite can carry stops its winner's rename at the primitive. | A rebuild keeps what it did not touch: the older version is the user's data, and the rule keeps a member and its history together. An `mv` that loses what a rename inside the archive keeps would change the rule by where the target lies. | 11 |
| D-157 | **Writer's lock** (W5b review; extends D-148 and D-149). A batch holds an advisory lock on the sibling `.<name>.hikasa-lock` of the file it replaces, from its first moment to its end, through `Vfs::lock_writer`; a held lock is `Unavailable`, at once, and the message names the lock file, so that one nothing can take over can be found. The holder removes the file while it holds the lock, and only that file; the next writer takes over one a dead process left. Readers never touch it, and a folder or file system that cannot have one falls back on the stamp. `AtomicWrite::sync` splits the commit, so the last look at the stamp comes after the fsync. | Two processes both passed a last look at the file and one lost its write with exit 0, in 24 of 24 runs. The lock cannot lie on the archive: a rename moves it to the old inode, and on Windows it would keep readers out. | 11 |
| D-158 | **Temporary files of atomic writes** (W5b review; extends SES-053). Owner-only from before the first byte until the rename, when they get the replaced file's mode or the one they were made with; locked by their writer, until the rename on Unix and until just before it on Windows, so whoever holds the writer's lock can remove what a dead writer left and only that (exact temporary names, owner-only, lock free). A killed rebuild leaves its files until the next rebuild of that archive; Ctrl-C does not, since D-159. | A copy of a private archive was world-readable while it was written. A kill leaves an archive-sized hidden file. | 11 |
| D-159 | **Ctrl-C in the CLI** (M1 Appendix C 15, plan §5.3). `ctrlc` 3 in `hikasa-cli` only, named only in `interrupt.rs`. The first job of a command arms the handler (`Ctx::job`), so a command without a job keeps the OS's behaviour: Ctrl-C ends it at once. All jobs of the process share one `CancelToken`: the first Ctrl-C cancels it, on a terminal with a line saying a second one exits at once; a rebuild aborts and removes its temporary and lock files, a `vfs` operation stops and reports its `not_run` items, a comparison stops at its next check. The command ends through its usual path with **exit 100**: `ApiError::Cancelled` and `VfsOpDto::ok() == false` already gave it, and no new code joins CLI-004. A folder comparison keeps what it saw for the GUI, and `Ok` with an incomplete model would print the verdict of that part, so the adapter checks the token before it renders: `--output` stays as it was. A process that inherits SIGINT as ignored (`cmd &` without job control) stays that way: the handler goes in with `try_set_handler`, over the default action only. The second Ctrl-C exits 100 where the process stands, as a kill would (D-158 covers what it leaves). No `termination` feature: SIGTERM and SIGHUP stay as they were. A Ctrl-C after the job ended, while the result prints, is not seen until the second one. | The handler belongs to the adapter, core only sees the token (T1, T11). A job that cannot see the cancel, such as one blocked on standard input, must not trap the user: hence the second press. | 12 §4, 02 |
| D-160 | **Named fonts** (revises D-113, D-133). `fontique`, already in `Cargo.lock` through eframe's system font fallback, becomes a direct dependency of `hikasa-gui` at the same version and features, named only in `fonts.rs`; no package is added. Each distinct non-empty family of the four slots is looked up by name (any case, the face nearest regular), on a thread of its own, one lookup at a time, the newest theme winning; `use_system` keeps the folder's family out. The answers come through the mailbox (`UiMsg::Fonts`); `Effect::ApplyFonts` rebuilds egui's definitions once when what the slots show changes, never for a colour or a family not found. Each slot is a named egui family: its font, then the whole built-in chain, then eframe's system fallback, so icons and CJK keep working. The enumeration is dropped after each lookup, only the files in use stay mapped (`FontData::from_blob`, no copy), and a family no slot names is forgotten. The Options dialog's preview looks families up as they are typed but reports a missing one only once applied. `FontSource` is injected: tests register font data and never see the system's fonts. | A font file is not a comparison input: this is the GUI's own look, so it stays out of `Service` and of D-105's list while keeping D-105's rule that the UI thread never reads a disk. Parsing is not enough: a file that parses can still panic where egui draws it, on the UI thread, at every start (a font of 0 units per em does; one with a broken table panics in the hinter). So a font with 0 units per em counts as not found, and each font that passes draws a sample in a throwaway egui context on the lookup's thread, under `run_guarded`; one that fails counts as not found, with the notice and the built-in font. A file can be broken at any glyph, and the sample is printable ASCII and some Latin: a panic at a glyph outside it still reaches the window. | 14 §2, 02 |
| D-155 | **G6** (§1.5.16). List 100,000 zip entries < 1 s; a 10,000-file tree against its zip by CRC < 2 s; the index of a 1 GiB `tar.gz` ≤ 1.3 × the raw decode, CRC included; G2 unchanged; RSS budgets. Headless, release, idle machine, fixtures generated; G4 holds for long decodes. | Plan §5.2 confirmed; the first figure already measured at 15 ms for the crate's part. | 02 |
| D-161 | **W6 in four sequential parts.** W6a *Foundations* (`ops` batch, stamps, durability, fence, backups, exchange, filters, Ignored, refresh, contents on demand), W6b *Operations* (selection, plans, confirmation, `folder op`), W6c *Sync* (`folder sync`, session type, G7), W6d *GUI hooks* (dialogs, four commands, refresh in the view, docs). Each ends green, with its own acceptance list. | Each part is additive and depends on the one before: W6b's plans need the batch, W6c needs the builder, W6d shows the plans. | 16 |
| D-162 | **One batch plan per operation** (T12, revises D-103's one operation per plan). `OpKind::Batch`, `BatchOptions`, `Action::Exchange`, `PlanItem` fields `expect`, `backup`, `times`, `follow_source`, `clear_read_only`, `trash_fallback`; `ops::execute_with(.., &ExecOptions, ..)`, of which `execute` is the default form. A folder operation or sync is one `ops::Plan`, executed on one thread. | A sync copies both ways and deletes; two `execute` calls would mean two rebuilds and two backups of an archive. The item runners already dispatch on `Action`. | 11 |
| D-163 | **A plan carries stamps; it is not re-planned** (hook of §1.5.19 W5b note 7). Items carry `Expect` (kind, size, time of source and target as the model saw them); `run_item` compares them with one `lstat` and fails `changed since the plan was made`. Stamps serialize, unlike `PlanItem::seen`. | Plans are made from the model, so a re-plan repeats stale facts unless it re-scans, which moves the items under an answered confirmation. A stamp costs no system call beyond D-103's re-check, and an archive's other members are safe because `begin` reads the archive's stamp itself. | 11 |
| D-164 | **Durability by case** (revises D-95). `Durability::{Full, FileOnly, None}` in `WriteOptions`, `Vfs::copy_new_file`. A new target: no `fsync`, metadata set on the handle before the rename. A replaced target: the file only. A move: `Full`, because a source goes on the strength of the copy. Saves, settings and rebuilds stay `Full`. `vfs cp` follows, as it is the same code. | Two `fsync`s per file made a copy of 10,000 small files 26 × slower than a plain copy (6.2 s against 240 ms), which rules out G7 whatever else is tuned. A power cut can now lose files a copy created, never one that existed. | 11 |
| D-165 | **Exchange** (FC-071) = `Action::Exchange`: copy left to a hidden sibling on the right, right to one on the left, then two renames. No backup. An orphan moves. Folder pairs exchange row by row. | Neither side can be lost at any step, across file systems and into archives; a failure before the third step leaves both sides as they were. | 04 |
| D-166 | **Fence** (plan acceptance: a mirror never deletes outside the bases). `ExecOptions::fence` refuses at execution any removal, source removal and (with `writes`) any write outside the bases, by the text and by the tree (no link from the base down to the item's folder); a base itself is never removed. The planner refuses what the fence cannot see: rows below a **followed** folder, and rows Unknown, Error or below a failed folder, which are never written or deleted. | The model may be old, and with FC-013 a path below a followed link leads elsewhere while its text stays inside the base. A failure is never a difference (D-19). | 04, 11 |
| D-167 | **Backups keep what a copy or move replaces** (FC §8, OPT-008). Not removes: a `.bak` left by a delete is an orphan the next mirror would delete and back up, for ever; deletes have the recycle bin (§1.3.10 named it there). Names are final at planning. New content is staged, the old file renamed to its backup just before the commit, renamed back if that fails; another disk → a copy first. `folder` naming keeps the path below the base. A failed backup fails the item. One generation on disk. | Costs one rename and no disk; the target is missing for one rename only, and nothing is replaced without its backup. A flat folder would let `a/x.txt` overwrite the backup of `b/x.txt`. | 04, 15 |
| D-168 | **One backup of the outermost file per operation; none per member** (§1.5.17). `ops/roots.rs::finish`, just before `end(root, Commit)`, for target and source-only roots, from `ExecOptions::archive_backups`, which the planner fills so a dry run shows it. A failure aborts that root. | A member cannot be renamed aside cheaply, and the archive holds them all; a rebuild rewrites the whole file anyway. | 11 |
| D-169 | **Attribute and Unix-type filters** (plan A5). `filters.other.attrib`, `.unix_type`, parsed by core from `+rh-s`-style text; `attrib` on regular files only, `unix_type` on everything but folders; `Entry::special` for `b c p s`. The content filter stays P1. | SCR-001's `filter` vocabulary is P0 although FC-053 is P1. A filter that hid folders would hide their subtrees; `EntryKind::Other` cannot tell a FIFO from a device. | 04, 12 |
| D-170 | **FC-012 keys.** `handling.auto_scan_subfolders` (true), `expand_on_load`, `expand_differences_only`; `scan_orphan_folders` keeps its meaning. Not scanned = `NotScanned`, verdict incomplete; a refresh scans it. | Defaults give M1's results; an unscanned folder claims nothing (D-19). | 04 |
| D-171 | **Ignored marks** (FC-033): `rules.ignored`, paths with `!` exceptions, longest path wins; `NodeFlags::IGNORED`; only `category` changes, not state, diffs or content. `FolderModel::set_ignored` is in place. Errors and unknown stay. | In the rules because they change the verdict, as `open_archives` does; the columns still show why. Exceptions keep the list short when one child leaves an ignored folder. | 04 |
| D-172 | **FF-002**: `rules.format_overrides` (id → bool), plain data in `hikasa-folder`; the Service turns it into `EnabledOverrides` for `RulesComparator` and for child views. | The M1 seam `with_overrides` existed; a child opens with the format its row was compared by. | 10 |
| D-173 | **Subtree refresh** (plan §5.3, §1.5.17). `refresh_into` re-lists one folder level, keeps rows whose facts are equal (with their subtrees, content and CRC), and descends only into rows that changed. Same rows: in place, ids stable. Else a new block at the arena's tail, kept children re-parented, `renumbered` returned. Dropped nodes are marked, not reused; a flat model re-runs. Property: equal to a fresh comparison. | The arena's children are contiguous, which readers rely on; a re-run costs a whole scan to learn a few rows, and a whole-subtree rescan would make a copy into a base a full scan. | 04 |
| D-174 | **Contents on demand** (FC-029 b, c; FC-040). `compare_contents`, `compute_crcs`, `apply_child_result` update the model in place; a repeated test counts once; a child's result applies only while the pair's stamps hold. | Same engines, budget and pool as a scan; a stale child result must not overwrite a row. | 04 |
| D-175 | **Selection engine** = `hikasa_folder::select`, one parser of the script's words (T11): masks OR-ed, `diff` = differences other than time, folders match only if all contents match, collapsed folders are one row. | SCR-002 as normative; O(rows) in one bottom-up pass, for G7 and W9's rows. | 04, 12 |
| D-176 | **Visible contents; `include_hidden_by_default` flips to false** (revises OPT-009). `select::resolve` is the one definition: a folder gives its visible rows, a child selected with its folder wins, `whole` means nothing is left out. The option keeps working, and each dialog and `--include-hidden` overrides it. | docs/04 §8 and plan W6 say filters are honoured by default, docs/15 says `true`: they cannot both hold. Acting on what the user cannot see is how data is lost. | 04, 15 |
| D-177 | **One builder, from the model** (T11, G7). Targets are the counterpart's own spec; an orphan goes under the other side's own folder; directory I/O only for a folder the model has no contents for; one `rename`, `trash` or `remove_dir` only for a `whole` subtree; `hikasa_vfs::ops` executes. | `x.txt` ↔ `x.csv`, NFC ↔ NFD and case pairs stay aligned, names that are not UTF-8 survive, and a plan of 100,000 items costs no I/O. | 04 |
| D-178 | **Confirmation is data**: `Conflicts` per item, `Confirmation` as a question machine, driven by the CLI, dialogs and scripts; a reply `YesToAll` is per class, a mode per plan; `NoToAll` lets clean items go. CLI without a terminal or `--yes` → exit 100, nothing touched. `delete` and a sync that deletes or replaces always ask, whatever `confirm_delete` says (D-77). | Yes to read-only must not mean yes to system. A script's safety must not depend on a settings file. | 04, 12 |
| D-179 | **Recycle bin** (FC-073, FS-005). `Recycle { use_bin, fallback }`. A spec without `TRASH` is the question `no_recycle_bin`; a `trash` that fails at execution fails the item unless `fallback` was chosen beforehand, then it expands into the permanent removals stamped at planning, never a listing at execution, with a warning. Off by default. | §1.3.10 made the fallback a confirmation choice; a silent permanent delete on a share is the worst way to learn a bin was missing. | 04 |
| D-180 | **Rename masks** (FC-074). `dos_rename`, `base_mask` by normative tables; regex over the whole name through `hikasa_formats::Pattern` (additive `captures_at`), so `regex` first and `fancy-regex` only for backreferences and look-around, with T9's limits; an in-house `$` expansion. A rename never replaces; chains are ordered, cycles broken by a hidden name. `hikasa-folder` gains `hikasa-formats`. | One engine policy (T11); the two crates expand `$1a` differently. | 04 |
| D-181 | **Sync matrix** (FS-002). Five presets as a normative table; a left-only item is `added_left` by default, `deleted_right` with `orphans = "deleted"`; invalid cells are refused; *different* under Update is a conflict, left alone. | FS-002 has eight rows, and without Sync Memory (P1) two are indistinguishable from two others: the user says which, and FS-003 later says per item. Nothing tells which of two different files wins. | 04 |
| D-182 | **Sync plan.** Deletes (post-order), then copies right, then left, one batch. Effective rules force scanning and `archives = Always` or `AsFiles` by *within archives*; flatten is refused. Empty folders follow `create_empty`; a folder wholly deleted goes. `EmptySource` is refused unless `--allow-empty-source`, which `--yes` does not imply; a mirror on an incomplete comparison is refused; error rows give `sync_incomplete`, exit 100. | A name freed by a delete may be needed by a copy, and deletes free the disk. A mirror from a failed mount would delete everything. | 04 |
| D-183 | **Sessions.** `SessionRules::Sync(SyncRules)` = `FolderRules` + `[rules.sync]`, replacing `Opaque`; `Specs::read_only_left/right`; schema version unchanged. | D-81's *until W6*. Per side, because `--ro1`, `--ro2` exist. | 03 |
| D-184 | **Disable editing is enforced in the Service** (FC-003). `ApiError::ReadOnlySide` (`permission_denied`) for a `folder op`; row by row for a sync; `CopyTo` out of a read-only side is allowed, a `copyto` or `moveto` whose target leads at or below a read-only base is not; child views carry the flag to W11. | Scripts and views cannot pass it by; a sync is previewed per row, a named operation is refused whole. | 04 |
| D-185 | **`folder op`, `folder sync`.** Rule flags shared with `folder compare`. No `--select` and no `--path` selects nothing and exits 100; `--path` names one row, never the base. `--display all` and everything expanded by default. Exit 0, 100, 107, as `vfs`. `hikasa.folder_op.v1`, `.folder_sync.v1`, `.folder_contents.v1`; `SCHEMAS` 38. | A stray command must not act on a whole tree. Failed items keep `ok: true` (W3). | 12 |
| D-186 | **Open With** (FC-062). `AppLauncher` injected, `SystemLauncher` adapter-only; `open` 5.4.5 for the associated application. `extract_to_cache` extends D-140: a numeric folder and a checked real name, read-only, kept for the session, never evicted. | A program needs a name and an extension. Tests never start a program. | 04, 11 |
| D-187 | **Find Filename and Compare To have no subcommand.** `Finder` and `open_pair` are pure and D-105 lets a view call them; `gui open` and `quick` are the headless forms. | Navigation needs a view; adding commands would duplicate existing ones. | 04, 12 |
| D-188 | **Operation log** (FC-100). `op_log_entries`, `sync_log_entries`: a summary, every failure, details by OPT-018 up to 10,000. | D-116's book is bounded, but a run should not push a million lines into it. | 04 |
| D-189 | **GUI of W6**: `CopyRight`, `CopyLeft`, `Delete`, `PermanentDelete` over a one-row selection, `PlanConfirmDialog`, `DeleteDialog`, log, notices, refresh with `renumbered`, `Open` on an archive by refresh, FC-012 on load, FC-029(c). `Del` and the caption of `PermanentDelete` enter the catalogue. | The dialogs of §1.4.18 need a way in; W9 owns the rest. | 14 |
| D-190 | **`--sync`, `folder sync --gui`** are W6 up to IPC: parsed, sent, `ViewRequest::Sync`; the window answers `unsupported` until W9. | The Folder Sync view is W9; CLI-003 and CLI-G03 are checked at the boundary. | 12 |
| D-191 | **Dependencies.** `open` 5.4.5 in `hikasa-session`; `hikasa-folder` += `hikasa-formats`. Check 2 and 7 rules. | `open` was approved in plan §3; §0.4's *W7/W13* is overtaken by FC-062. | 02 |
| D-192 | **G7.** Copy of 10,000 files ≤ 1.5 × a plain recursive copy that keeps times, on a prepared plan; sync plan of 100,000 files < 0.5 s. A failure is reported, not relaxed. | A scratch run of §1.6.3's sequence gave 1.45 ×. The comparison is not part of the gate, as its cost is G2's. | 02 |
| D-193 | **W7 in two sequential parts.** W7a *Core* (the W6 `copyto` defect, `ReportRequest` and the vocabulary, interleaved, XML dumps and their schema, HTML wrap and the print block, link rendering and names, streaming), W7b *Service and CLI* (pair requests in core, write plans and linked reports on disk, targets, report time, remembered choices, `ReportArgs`, `hikasa report`, `hikasa.report.v1`, help, docs, G13). Each ends green with its own acceptance list. | W7b renders W7a's layouts; the defect is in `hikasa-folder` and goes first, so P-OPS is green before the wave's own properties start. | 16 |
| D-194 | **An aligned pair of folders is one folder at a `copyto`/`moveto` target** (W6 defect carried over, FC-072, §1.7.2). With structure `base` or `relative`, the target folder of a folder pair whose names differ (case, NFC/NFD) is chosen once: an existing folder named as the left spells it, else as the right does, else the folder the plan makes, named as the first source side in plan order (left before right); every item below the pair, of either side, goes under it. File pairs, structure `none`, `copy`, `move` and `exchange` are unchanged. | `l/D/x.txt` and `r/d/y.txt`, aligned case-insensitively, planned `/o/t/D` and sent the right file to `/o/t/d/y.txt`, which failed `not_found` on a case-sensitive target: the plan made one folder and wrote into two. A second run must land where the first did, hence the existing folders first. | 04 |
| D-195 | **Scope of the report ids** (§1.7.3). XML dumps and links to file reports are M2 (RPT-001, RPT-002 are P0; plan W7): docs/16's M3 line "reports for every view incl. XML and linked file reports" is read as the P1 views' reports and reworded. FC-079 *File Compare Report…* over a selection (P1, docs/04) wins over RPT-001's sentence for that command; the script's `file-report` with two specs is a text report. TP-003 (P1) stays done. W7 adds no GUI command. | P0 before P1 (docs/17 §1); docs/04 owns the folder view's commands and no wave lists FC-079; the roadmap line predates the plan. | 13, 16 |
| D-196 | **One parser of the report vocabulary** (SCR-001, T11; §1.7.4). `ReportRequest::parse(kind, words, defaults)` reads the script's `layout:`, `options:`, `output-options:` and every CLI report flag (turned into words by core) through one slot table; two different words of a slot are a conflict, a word of the other kind or an unknown one is `invalid_input`, a word the layout does not use is kept with one warning `report_option_unused`. Layout × format × target is checked before the comparison runs. SCR-001 gains `no-line-numbers` and `display-none` (RPT-003's *None*). `words()` is canonical and round-trips. | Scripts (W8), the dialog (W13) and the CLI must mean the same by the same word; the dialog's remembered words span layouts; M1 refused `--report patch --html` only after comparing (M1 §1.9.3 item 10a). | 12, 13 |
| D-197 | **Interleaved layout** (§1.7.5). One column in row order: a same row once with both numbers, a difference as its left lines then its right lines, hidden runs as one row. Text: `<left no> <right no> <mark> <text>`, a two-character mark (side `-`/`+`, importance `!`/`~`); HTML: a table classed as side-by-side. Strike-out of a side strikes its inline differences and its orphan lines: `<del>` in HTML, SGR 9 with ANSI, `[-…-]` in plain text. Wrap, width, line numbers and display modes as side-by-side. | docs/13 names the layout and its options without a shape; reusing side-by-side's cells, escapes and classes keeps one colour semantics (RPT-010). Plain text cannot strike; `[-…-]` is the usual word-diff mark. | 13 |
| D-198 | **XML dumps** (RPT-002, §1.7.6). One document type: root `hikasa-report`, namespace `urn:hikasa:report:1`, `version="1"`, `type`; a header carrying RPT-004; text: one `row` per comparison row with both texts and inline `d`/`u`; folders: nested `folder`/`entry` with every column per side; a `summary` of `count`s from the one `summary_counts`. UTF-8, `\n`, two-space indent, fixed attribute order; `<c x="…"/>` for a character XML 1.0 cannot carry, `&#13;` for CR, `raw-name` for a lossy name. Version 1 grows only by optional parts; a change of meaning is version 2 with a new namespace. Schema `docs/schemas/hikasa-report-1.xsd`, normative. | A dump is for programs: lossless, nested because XPath and XSLT walk trees (the JSON stays flat for `jq`, D-29), one line per row so it streams and diffs. | 13 |
| D-199 | **XML is checked twice, without a runtime crate** (§1.7.7). Every test parses its output with `roxmltree` 0.21.2 (dev-dependency of `hikasa-report`, the version `Cargo.lock` holds through `fontique`: no new package) and checks the structure; goldens and generated documents are validated by `xmllint --noout --schema`, required on Linux and macOS CI (`HIKASA_REQUIRE_XMLLINT=1`; the Linux job installs `libxml2-utils`), skipped with a note on Windows; a negative case proves the check can fail. | No maintained pure-Rust XSD validator exists; libxml2 is the reference and is used as a tool, never linked. A schema checked only by the code that writes it proves nothing. | 02, 13, 17 |
| D-200 | **Links to file reports** (RPT-001, §1.7.8). Folder side-by-side HTML only. Every shown file pair present on both sides whose category is newer or different gets a side-by-side HTML text report with the folder report's palette, compared through `Service::pair_text_request` (the GUI's child request, moved to core: text rules, format overrides, ignore-unimportant) and the content budget (D-174); pairs over the rules-based size limit or binary are listed as skipped. Files `<safe stem>_files/<n>-<safe name>.html`; hrefs relative, `/`-separated, percent-encoded; each file report links back. | Numbers keep names distinct on case-insensitive file systems; safe names stay readable and portable (Windows device names, `#`, `%`, controls); relative links survive moving the report with its folder. | 13 |
| D-201 | **A linked report is written by a plan** (T12, §1.7.9). The main file is the one named (replaced as `--output`); its files folder only when absent or **ours** (a marker `.hikasa-report.toml` names the main file and stamps the files it wrote, and the folder holds exactly those); else the first free `<…>_files-<k>`, warning `report_folder_renamed`. Nothing not ours is written, replaced or removed. Execution: pair files into a hidden staging sibling, the main file into its temporary file, then the old folder aside, the staging folder in, the main file committed, the old folder removed; a failure or cancel before the end restores everything. `--dry-run` prints the plan; durability by D-164's cases. | A report that writes thousands of files must not overwrite what the user did not ask for, and a re-run should replace its own output rather than pile up copies; the renames give a folder `--output`'s all-or-nothing. | 12, 13 |
| D-202 | **Reports stream** (§1.7.9). A file or stdout target renders through a 64 KiB buffer straight into the VFS's atomic writer (`write_output_with`); a `CancelWriter` checks the token every MiB, so renderers stay pure. Only the clipboard holds a report, at most 128 MiB (`too_large`). `write_output` stays as a wrapper. | A 1M-line side-by-side HTML report is ≈ 280 MB, which M1 rendered into a `Vec` first; G4 holds without a `Ctl` in every renderer. | 12 |
| D-203 | **Clipboard target** (§1.7.10). The Service returns a `ClipboardPayload` (text; for an HTML layout the HTML plus the same layout as plain text); the adapter writes it through the new `ClipboardProvider::write_payload` (`SystemClipboard`: arboard `set_text`/`set_html`; `FileClipboard`: a file target's bytes; `FixedClipboard` records). The Service never writes the clipboard; links are refused. On Linux (X11, Wayland) the content outlives `hikasa`: `SystemClipboard` hands the payload to a helper process (`hikasa` through a hidden internal entry) that owns the selection with arboard's wait mode until another program takes it, then exits; no new runtime dependency (maintainer's order after the first text). | The plan's *byte payload returned to the adapter*: the GUI writes its own clipboard, the CLI's arboard handle belongs to the adapter, and tests read `HIKASA_CLIPBOARD_FILE`. | 12 |
| D-204 | **Printer and browser** (Q6 = a, D-62; §1.7.11). Both write the HTML report and its files into a fresh `<cache>/print/<date>-<time>-<n>/` and open it through `AppLauncher::open_associated` (D-186); `printer` adds the print block (orientation, print palette). Folders there older than seven days are pruned at the next such report. Layouts without HTML are refused; no native printing. W13's *Print Preview* and *View in Browser* are these targets. | The maintainer chose the browser for printing; a folder per report keeps links and never races another print. | 13 |
| D-205 | **`hikasa report text\|folder`** (§1.7.15). Specs or `--session`, the compare subcommand's rule flags, and the shared `ReportArgs` (`--layout`/`--report`, `--options`, `--output-options`, `--output-to`, `--title`, M1's report flags, `--lines` or `--select`/`--path`, `--dry-run`), which `text compare`, `folder compare` and `session run` flatten too. Default layout side-by-side, every other default the compare subcommand's, so `report text a b` ≡ `text compare a b --report side-by-side` byte for byte. Exit code: the comparison's. `--json`: `hikasa.report.v1` (`SCHEMAS` 39), refused with the stdout target. | The subcommand is the script commands' CLI form (W8 maps one to one) and where W12's types go; the shortcut stays. One flag group, one owner. | 12 |
| D-206 | **Report time** (extends D-24; §1.7.12). One instant per report job, shared by the folder report and its file reports: `SOURCE_DATE_EPOCH` when set (seconds, shown in UTC; captured by `capture_env`), else the clock in the user's zone; a bad value warns `source_date_epoch_invalid`. | Reproducible reports for archives and builds; tests compare the CLI and the Service byte for byte without stripping the header. | 12, 13 |
| D-207 | **RPT-011 is proven by bytes** (§1.7.12). For one comparison and one request, the CLI's `report`, the shortcut, the Service, stdout, a file, the clipboard payload and the printer and browser files (with the words those targets imply) are byte-identical, for every layout × format × kind. | M1 asserted RPT-011 by construction only. | 13 |
| D-208 | **Remembered choices** (RPT-003, §1.7.13). `state.toml` `[dialogs."report-text"]`, `[dialogs."report-folder"]`: `layout`, `options`, `output_options` (words), `output_to`, `file`; `Service::report_choices`, `remember_report_choices` through `state_update` (D-108); a word the parser refuses is dropped with a warning. The CLI never reads them. | As words, D-196's parser reads them back; a command line must not depend on what a dialog did last. | 03, 13 |
| D-209 | **Report scope** (RPT-003 *Just selection*, §1.7.4). `ReportScope::{All, TextLines, FolderRows}`, set by the Service: text rows between the first and last whose line on a side lies in `--lines [left:\|right:]a-b`; folder rows of W6's `--select`/`--path` with what lies below and their ancestors. Row layouts only; summary, statistics, patch and JSON refuse it. The header names the scope. | The dialog's checkbox must be reachable headless (docs/17 §2 rule 2b); counts or a patch of part of a comparison would mislead. | 13 |
| D-210 | **HTML wrap and the print block** (RPT-002, §1.7.4). `wrap-none`, `wrap-character` (M1's rule, bytes unchanged), `wrap-word` set one CSS rule of text HTML; `print-portrait`/`print-landscape` (or the printer target) add `@page` and `@media print` with the print palette (`print-color`/`print-mono`). Without these words every HTML byte is M1's. | RPT-002 lists wrap for HTML and the printer; M1's goldens stay valid. | 13 |
| D-211 | **Dependencies** (§1.7.16). No runtime crate. `roxmltree` 0.21.2 (MIT OR Apache-2.0) as a dev-dependency of `hikasa-report`, at the version already in `Cargo.lock`; `xmllint` as a CI tool. Check 3: `hikasa-report` dev += `roxmltree`; check 2: `write_payload` only in the clipboard module, `host.rs` and the adapters; check 7: the adapters call no report slot parser, and `ReportRequest::parse` only `hikasa_session::api::report`. | Plan §3 adds no crate for W7; D-199. | 02 |
| D-212 | **G13 reports** (§1.7.17). Text: each new layout of the G1 pair to a file ≤ the summary run + 1.5 s, peak RSS ≤ + 24 MiB; folder: the G2 tree as side-by-side HTML and XML ≤ + 1.0 s, RSS ≤ + 32 MiB; links: 1,000 file reports ≤ 5 s, RSS ≤ + 64 MiB; streaming proven on CI by a counting allocator. A failure is reported, not relaxed. | The plan set no report gate; streaming is the property that matters and is cheap to prove on every run. | 02, 12 |
| D-213 | **Pieces per side in the text XML** (§1.7.6, W7a review). A `row` carries `left-piece` and `right-piece`, each only when that side's line is piece 2, 3, … of a line broken by the characters-per-line limit; the single `piece` (the larger of both) is gone. The schema is unreleased, so version 1 changes in place. | With a limit, one side's line can be a piece while the other's is whole (a long left line beside two short right lines): one attribute made a reader join the right side's whole line to the line before. | 13 |

### 0.4 Dependencies

W0 and W1 add no third-party crate (the change stamp uses `hikasa_vfs::crc32_reader`). Member manifests (architecture check 3): + `xtask` = `[hikasa-session]`. The
approved crates of D-63 enter the table of the wave that first uses them. **W2** adds `zip` (D-63) to `hikasa-session` for settings packages, with the workspace-level features `deflate` only (pure-Rust back-end; W5 adds the archive features); `docs/02` gains its line.
**W3** adds `trash` 5 to `hikasa-vfs` (`default-features = false`, features `chrono`, `coinit_apartmentthreaded`;
called only from `local/trash.rs`) and `arboard` 3 to `hikasa-session` (`default-features = false`, features
`image-data`, `wayland-data-control`, the set `Cargo.lock` already compiles through `egui-winit`; named only in
`host.rs`). Member manifests: `hikasa-vfs` += `trash`, `hikasa-session` += `arboard`; check 2 gains the two
confinement rules of §1.3.9; `docs/02` gains both lines; `cargo deny` must pass without new exceptions (D-98,
D-100).
**W4** adds no dependency (D-133): icons are drawn with egui's painter (D-114), fonts are egui's built-ins (D-113, until
D-160), and `open` waits for W7/W13; `Cargo.lock` gains no package and the member manifests of check 3 are unchanged. The
architecture test gains one check-7 rule (§1.4.16 rule 4).
**W5** (D-154) moves `zip` to 9.2.0 with the archive features and adds `tar` 0.4.46, `flate2` 1.1.10, `bzip2` 0.6.1, `lzma-rust2`
0.23.0 and `zstd` 0.14.3 to `hikasa-vfs`, with `jiff` for the injected zone: §1.5.15 has the exact entries. Member manifests:
`hikasa-vfs` += `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `zstd`, `jiff`. `lzma-rust2` and `zstd` follow `zip`'s requirements, so one
copy of each is in `Cargo.lock`. `deny.toml` gains one crate-scoped exception (`libbz2-rs-sys`, `bzip2-1.0.6`), the only new licence;
`zstd-sys` brings the only C code, built by the `cc` crate with the target's C compiler; check 2 gains one confinement rule.
**W6** (D-191) adds `open` 5.4.5 to `hikasa-session` (`default-features = false`; named only in `host.rs`; it was to wait for W7/W13, but FC-062's
associated application is W6) and gives `hikasa-folder` the edge to `hikasa-formats`, for `Pattern` (D-180). Member manifests: `hikasa-session` += `open`,
`hikasa-folder` += `hikasa-formats`. `cargo deny` must pass without new exceptions; check 2 and check 7 gain the rules of §1.6.21.
**W7** (D-211) adds no runtime crate: `roxmltree` 0.21.2 becomes a dev-dependency of `hikasa-report` at the version `Cargo.lock`
already holds through `fontique` (no new package), and `xmllint` is a CI tool, never linked. Member manifests: `hikasa-report` dev +=
`roxmltree`; check 2 and check 7 gain the rules of §1.7.16.

### 0.5 Documentation updates of W0

`docs/01` §3.6 (catalogue in core), `docs/02` §4 (tooling), `docs/12` §1 and §4 (`help`), `docs/14` (F1 panel),
`docs/16` (M1 leftovers, M2 progress), `docs/17` §3–§4, `CHANGELOG.md`. The OPT ids (D-61) and the M3 list of
D-59 are written in W1, where the options are implemented.

---

## 1. Wave contracts

### 1.0 W0 — Infrastructure (complete)

**Files.** `core/hikasa-session/catalogue/commands.toml` (data, D-65); `core/hikasa-session/src/commands.rs`;
`core/hikasa-session/src/help.rs`; `help/*.md` (17 topics: `index`, `cli`, `exit-codes`, `json-output`, one per
M1 subcommand and `help`, `main-window`, `view-home`, `view-folder-compare`, `view-text-compare`);
`cli/hikasa-cli/src/cmd/help.rs`; `gui/hikasa-gui/src/ui/help.rs`; `xtask/src/main.rs`; `.cargo/config.toml`;
`.config/nextest.toml`; `.github/workflows/ci.yml` and `bench.yml`.

**`hikasa_session::commands`.**

```rust
pub const CATALOGUE_SCHEMA_VERSION: u32 = 1;
pub enum CommandStatus { Implemented, Planned }          // serde snake_case; as_str()
pub struct CommandEntry { pub id: String, pub menu: String, pub caption: String, pub shortcut: String,
                          pub description: String, pub status: CommandStatus }   // deny_unknown_fields
pub struct CatalogueSection { pub key: String, pub title: String, pub help: String, pub note: String,
                              pub commands: Vec<CommandEntry> }
pub struct Catalogue { pub schema_version: u32, pub title: String, pub intro: String,
                       pub sections: Vec<CatalogueSection> }
pub struct CatalogueError(pub String);
pub fn catalogue() -> &'static Catalogue;                // parsed once; empty and logged if invalid (T9)
pub fn try_catalogue() -> Result<Catalogue, CatalogueError>;   // parse + problems()
pub fn parse_catalogue(text: &str) -> Result<Catalogue, CatalogueError>;
impl Catalogue {
    pub fn section(&self, key: &str) -> Option<&CatalogueSection>;
    pub fn find(&self, keys: &[&str], id: &str) -> Option<(&CatalogueSection, &CommandEntry)>;
    pub fn entries(&self) -> impl Iterator<Item = (&CatalogueSection, &CommandEntry)>;
    pub fn problems(&self) -> Vec<String>;
    pub fn render_markdown(&self, status_column: bool) -> String;
}
```

Problems: a section key not `[A-Za-z0-9-]+` or used twice; an id not `[A-Za-z0-9]+` or listed twice in its
section; a `help` topic that does not exist; a menu, caption, shortcut or description with `|`, a line break or
surrounding spaces.

**`hikasa_session::help`.**

```rust
pub struct HelpTopic { pub id: &'static str, pub source: &'static str }
pub const INDEX_TOPIC: &str = "index";
pub fn topics() -> &'static [HelpTopic];
pub fn topic(id: &str) -> Option<&'static HelpTopic>;
pub struct UnknownTopic { pub word: String, pub suggestions: Vec<&'static str> }   // Error
pub fn resolve(words: &[String]) -> Result<&'static HelpTopic, UnknownTopic>;
pub fn suggestions(word: &str) -> Vec<&'static str>;
pub enum Inline { Text(String), Code(String), Link { label: String, topic: String } }
pub enum Block { Heading { level: u8, text: Vec<Inline> }, Paragraph(Vec<Inline>), Item(Vec<Inline>),
                 Verbatim(String) }
pub fn parse_inline(line: &str) -> Vec<Inline>;
pub fn parse_blocks(source: &str) -> Vec<Block>;
pub fn plain(inline: &[Inline]) -> String;
impl HelpTopic { pub fn title(&self) -> String; pub fn blocks(&self) -> Vec<Block>;
                 pub fn links(&self) -> Vec<String>; pub fn render_text(&self) -> String; }
pub fn problems() -> Vec<String>;
```

Terminal text: the title underlined with `=`, one blank line between blocks (none between consecutive list
items), items as `- …`, verbatim blocks indented by four spaces, links as their label, code in backquotes.

**CLI and JSON.** `hikasa help [TOPIC…] [--list]` (`--list` conflicts with a topic); exit 0, an unknown topic 100
with `error.kind = "invalid_input"`. `HELP_SCHEMA = "hikasa.help.v1"` is appended to `SCHEMAS` (19 schemas).
`help` runs without the service, like `version` and `completions`. DTOs: `HelpDto { topic: Option<HelpTopicDto>,
topics: Vec<HelpListItemDto> }`, `HelpTopicDto { id, title, markdown, links }`, `HelpListItemDto { id, title }`.

**GUI.** Catalogue entries `HelpContents` and `ContextHelp` (F1, global scope). `AppCore::{help, show_help,
close_help}`; `Scope::{catalogue_sections, help_topic}`. `ui::help::show` draws the panel and only reports
`Open(id)` or `Close`.

### 1.1 W1 — Settings store

**Ids.** SES-051, SES-052, SES-053, FF-001 (CLI), FF-004 (write paths, restore, detection), UI-003 (persistence),
UI-020…023 (scheme files), SES-041 and SES-091 (options), OPT-001…OPT-022 (`docs/15` "Keys"). No GUI change in
W1 except keeping it compiling (the GUI reads the new model in W4).

**Files.** `core/hikasa-session/src/settings/` (`mod.rs`, `file.rs`, `stamp.rs`, `keys.rs`, `options.rs`,
`state.rs`, `commands.rs`, `theme.rs`, `filters.rs`, `openwith.rs`, `archives.rs`), `core/hikasa-session/src/
formats.rs` (write paths), `core/hikasa-session/src/api/settings.rs` (Service calls and DTOs),
`cli/hikasa-cli/src/cmd/settings.rs`, `cli/hikasa-cli/src/cmd/format.rs`, `help/settings.md`, `help/format.md`
(extended), golden files. `options.rs` at the crate root keeps its public items (`ProgramOptions`,
`OptionsLoad`, `load_options`, `parse_options`, `OPTION_SECTIONS`, `OPTIONS_FILE`, `OPTIONS_SCHEMA_VERSION`) as thin
wrappers over `settings::options`, so M1 callers do not change.

#### 1.1.1 Module map

```rust
pub mod settings {
    pub use file::{SettingsFile, SettingsSchema, Overwrite, LoadMode, Loaded, Problem};
    pub use stamp::{ChangeStamp, ConfigWatch, ConfigChange, ChangeKind};
    pub use keys::{SettingsKey, KeyInfo, ValueKind, FileId, parse_key};
    pub use options::Options;            // every OPT key; `Options::program_options() -> ProgramOptions`
    pub use state::State;
    pub use commands::{CommandOverrides, ViewCustomization, CommandOverride, Shortcut, ShortcutConflict};
    pub use theme::{ThemeSelection, Fonts, FontSpec, Scheme, Colour, SyntaxStyle, ThemeMode};
    pub use filters::{FilterPresets, FilterPreset};
    pub use openwith::{OpenWith, OpenWithApp, WorkingFolder, ExpandContext, ExpandItem, ExpandError,
                       split_words, expand_command, InstanceMode};
    pub use archives::{ArchiveTypes, ArchiveType};
    pub struct SettingsStore { /* ConfigLocation */ }
}
```

`SettingsStore::new(&ConfigLocation)`; `store.options(mode)`, `store.state(mode)`, `store.commands(mode)`,
`store.theme(mode)`, `store.scheme(name, mode)`, `store.filters(mode)`, `store.openwith(mode)`,
`store.archives(mode)` return `Result<Loaded<T>, SessionError>`; `store.path(FileId) -> PathBuf`;
`store.watch() -> ConfigWatch` over every settings file plus `formats/` (each `*.toml` and `order.toml`).

#### 1.1.2 `SettingsFile<T>` (D-70, T13)

```rust
pub trait SettingsSchema: Serialize + DeserializeOwned + Default + Clone + PartialEq {
    const FILE: FileId;                 // which file (FileId::Themes(name) for schemes)
    const VERSION: u32;                 // current schema_version (all files: 1 in W1)
    /// Rewrites a raw table of `from` (< VERSION) into the shape of `from + 1`. Called once per step.
    fn migrate(from: u32, table: &mut toml::Table) -> Result<(), String> { Err(format!("no migration from {from}")) }
    /// Range, enum, mask, reference checks after deserialization; each Problem names a dotted key.
    fn validate(&self) -> Vec<Problem>;
    /// Repairs (LoadMode::Lenient): replaces each invalid value by its default.
    fn repair(&mut self, problems: &[Problem]);
    /// Top-level keys that are not sections of this schema but are allowed (none by default).
    fn known_sections() -> &'static [&'static str];
}
pub enum LoadMode { Strict, Lenient }
pub enum Overwrite { IfUnchanged, Force }
pub struct Problem { pub key: String, pub message: String }
pub struct Loaded<T> { pub value: T, pub stamp: ChangeStamp, pub migrated_from: Option<u32>,
                       pub warnings: Vec<Warning>, pub ignored_keys: Vec<String> }
pub struct SettingsFile<T> { path: PathBuf, raw: toml::Table, loaded: Loaded<T> }
impl<T: SettingsSchema> SettingsFile<T> {
    pub fn load(path: &Path, mode: LoadMode) -> Result<SettingsFile<T>, SessionError>;
    pub fn parse(text: &str, origin: &str, mode: LoadMode) -> Result<SettingsFile<T>, SessionError>; // pure core
    pub fn value(&self) -> &T;   pub fn value_mut(&mut self) -> &mut T;
    pub fn stamp(&self) -> &ChangeStamp;
    pub fn to_toml(&self) -> String;                          // the bytes save() would write
    pub fn save(&mut self, overwrite: Overwrite) -> Result<bool, SessionError>;   // false: nothing to write
    pub fn changed_on_disk(&self) -> Result<bool, SessionError>;
}
```

Load algorithm (normative; a test per step):

1. Missing file → `T::default()`, stamp `Absent`, `raw` empty. Other read errors → `SessionError::Io`. Bytes that
   are not UTF-8 → `SessionError::Config`.
2. Parse to `toml::Table` (syntax error → `Config`, message on one line as in M1).
3. `schema_version`: absent → `T::VERSION` (M1 files without it stay valid); negative or not an integer →
   `Config`; newer than `T::VERSION` → `SessionError::Schema` (SES-052), in **both** modes.
4. Older: apply `T::migrate(v, …)` for `v = found … VERSION-1` on the raw table; record `migrated_from`. A
   migration error → `Config`. The migrated file is not written at load (SES-052: written back on next save).
5. Unknown top-level keys (not a field of `T`, not in `known_sections()`, not `schema_version`) → warning
   `unknown_config_key`, kept in `raw`; unknown keys inside known tables → `ignored_keys`, kept in `raw`.
6. Deserialize `T` from the table **with unknown keys removed** (serde must not reject them; types use
   `#[serde(default)]`). A type error: Strict → `Config` naming the dotted key; Lenient → the key is replaced
   by its default value in the table, warning `invalid_config_value`, retry.
7. `validate()`: Strict → the first problem is `Config` (message `<origin>: <key>: <message>`); Lenient →
   `repair()`, one `invalid_config_value` warning per problem.

Save algorithm (normative):

1. If `overwrite = IfUnchanged` and the current disk stamp differs from `loaded.stamp` → `SessionError::Conflict
   { path }` (new variant; classifies as `invalid_data`; message `<path>: changed on disk since it was read`).
   The CLI never forces except `settings set --force`.
2. Document = `raw` with every known key path of `T` removed, then the **sparse** serialization of `value`
   merged in: a leaf (or a whole array) equal to the same path of `T::default()` is omitted; an empty known table
   is omitted. Arrays of tables (`[[app]]`, `[[preset]]`) are whole values; each element type keeps its unknown
   keys in a `#[serde(flatten)] extra: toml::Table` field, so they survive too.
3. `schema_version = VERSION` is always the first line. Sections in the order of the type's fields, then unknown
   sections in their original order. `toml::to_string` formatting; no comments.
4. If the document equals the file's current bytes → return `Ok(false)`, no write. If the document holds only
   `schema_version` and the file is absent → `Ok(false)` (never create a file of defaults).
5. Create the parent folder (`create_dir_all`), `write_atomic`, refresh `loaded.stamp`, return `Ok(true)`.

#### 1.1.3 Change stamps (D-74)

```rust
pub enum ChangeStamp { Absent, Present { len: u64, modified_ns: Option<i128>, crc32: u32 } }
impl ChangeStamp { pub fn of(path: &Path) -> Result<ChangeStamp, SessionError>; pub fn of_bytes(bytes: &[u8], modified_ns: Option<i128>) -> ChangeStamp; }
pub enum ChangeKind { Created, Changed, Removed }
pub struct ConfigChange { pub path: PathBuf, pub kind: ChangeKind }
pub struct ConfigWatch { /* path → stamp */ }
impl ConfigWatch {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> ConfigWatch;   // stamps taken now
    pub fn track(&mut self, path: PathBuf);                               // stamp taken now
    pub fn poll(&mut self) -> Vec<ConfigChange>;                          // sorted by path; updates stamps
    pub fn acknowledge(&mut self, path: &Path);                           // after our own save
}
```

`poll` compares `len` and `modified_ns` first and computes the CRC only when they are equal to the stored values
**and** the file was modified less than 2 s before the previous poll (same-second edits), else when they differ;
an unreadable file is reported once as `Changed` and keeps its old stamp. The `formats/` folder is tracked by
listing it at each poll (new and removed `*.toml` files are `Created`/`Removed`). No thread, no timer (D-62).

#### 1.1.4 Schemas

`options.toml`: `docs/15` "Keys" (binding; OPT-001…OPT-022; one Rust field per key, nested structs per table).
`archives.toml`, `commands.toml`, `openwith.toml`, `filters.toml`: `docs/15` "Keys". Validation beyond types:
masks parse with `hikasa_core::mask::MaskList` (OPT-010, OPT-020); filter preset names unique and non-empty;
Open With `description` and `command` non-empty and the command splits (D-76); a `commands.toml` view key is one
of the eight view names, a command id exists in one of the catalogue sections of that view (mapping below), a
shortcut parses. Unknown ids or views are **problems**, not unknown keys (a typo must not pass silently).
*Revised in W4c (§1.4.20):* in a loaded file they are kept as they are with a warning (`unknown_config_key`), as is the
override of a `planned` command with a shortcut this build cannot read (`invalid_config_value`): the problem stays for a
value built by hand and for a `settings set` that would create one.

View → catalogue sections (moves from the GUI's `Scope::catalogue_sections` into
`hikasa_session::commands::view_sections(view) -> &'static [&'static str]`; the GUI calls it):
`home` → `home, global`; `folder-compare` → `folder-compare, folder-shared, session, global`; `folder-sync` →
`folder-sync, folder-shared, session, global`; `text-compare` and `text-edit` → `text-compare, file-shared,
session, global`; `table-compare` → `table-compare, file-shared, session, global`; `hex-compare` →
`hex-compare, file-shared, session, global`; `picture-compare` → `picture-compare, session, global`.

**Shortcuts (UI-003).** `Shortcut::parse(&str)`: modifiers `Shift+`, `Ctrl+`, `Alt+`, `Cmd+` in any order
(case-insensitive), then one key: a letter, a digit, `F1`…`F24`, `Num 0`…`Num 9`, `Num +`, `Num -`, `Num *`,
`Num /`, or one of `Enter Tab Esc Space BkSp Del Ins Home End PgUp PgDn Up Down Left Right` or a single
punctuation character. A trailing context in parentheses (`Enter (in a path field)`) is kept as text and
ignored for conflicts. `Display` writes the reference spelling: `Shift+Ctrl+Alt+Cmd+Key`. Every non-empty
`shortcut` of the catalogue must parse (catalogue problem from W1). Effective shortcuts of a view = catalogue
defaults of its sections, replaced per command by an override. `CommandOverrides::conflicts(view) ->
Vec<ShortcutConflict { shortcut, commands: Vec<String> }>`: two commands of the same view with the same
shortcut (context-less) — reported by `settings list` as warnings and by the W4 editor; not a load problem
(the catalogue itself has view-dependent duplicates that the GUI resolves by focus).

`state.toml` (SES-051; write to `docs/03` §5.2):

```toml
schema_version = 1
last_session = ""                        # name of the session last used (W2)
[mru]                                    # each list newest first, capped at max_mru_items on push
left = []  ;  right = []  ;  center = []  ;  folders = []  ;  files = []
find = []  ;  replace = []  ;  name_filters = []  ;  workspaces = []
[window]                                 # last main window (a workspace stores its own, W2)
width = 1200  ;  height = 800  ;  x = -1  ;  y = -1  ;  maximized = false     # x/y −1 = let the platform place it (A17)
[flags]
first_run_done = false                   # set by the GUI first start (D-21 materialization)
hints_seen = []                          # ids of one-time hints
[dialogs]                                # free tables per dialog id (RPT-003 "remembers last choices"); kept as-is
```

`State::push_mru(list, value, max)`: removes an equal entry (exact string), inserts at the front, truncates to
`max`. Window sizes `200..=32767`, positions `-1..=32767`.

`theme.toml` and `themes/<name>.toml` (UI-020…023; write to `docs/14` §2.1):

```toml
# theme.toml
schema_version = 1
light_scheme = "light"
dark_scheme = "dark"
[fonts.editor]   family = ""  size = 13.0     # "" = the built-in monospace; size 6.0..=72.0
[fonts.listing]  family = ""  size = 13.0
[fonts.hex]      family = ""  size = 13.0
[fonts.folder]   family = ""  size = 13.0  use_system = true

# themes/<name>.toml
schema_version = 1
base = "light"                                # light | dark; ignored for the built-in names
[folder]   # UI-021
default_text, background, folder_info, selection, inactive_selection_background, stripes, log_panel,
filtered_out, unknown, same, newer, older, orphan, different, left_change, right_change, mergeable, conflict
use_system_selection = false, use_system_log_colors = false, use_stripes = true,
color_folders_by_content = true, gradient_center_background = false
[file]     # UI-022
default_text, background, stripes, edited, saved, line_numbers, ignored, current_line, inactive_selection,
selection, important_text, important_text_background, important_line_background, unimportant_text,
unimportant_text_background, unimportant_line_background, replacement, orphan, output, left_change,
right_change, conflict, find_highlight, gutter, whitespace
current_line_only_over_under = false, use_stripes = false, use_system_selection = false
fallback = ["#…", …]                          # exactly 8 colours (syntax colours by element id, M1 slots 1…8)
[file.syntax.<element name, lower case>]      color = "#…", bold = false, italic = false
                                              # built-ins define: comment, string, character, keyword,
                                              # identifier, number, operator, directive
[picture]  # UI-023
background, same, similar, different, checker_light, checker_dark
```

Every colour element is required in a **resolved** `Scheme` (`Scheme::resolve(name, file: Option<&SchemeFile>)
-> Scheme`: built-in values, overlaid by the file's set elements; a non-built-in scheme overlays its `base`).
Built-in values: the M1 `gui/hikasa-gui/src/ui/theme.rs` palettes where an element exists there (folder
colour classes, text backgrounds, gutter, syntax slots → `fallback`), otherwise chosen by the implementer;
default text on background and on each line background has a contrast ratio ≥ 4.5:1 (UI-009; unit test).
`ThemeSelection::scheme_for(mode: ThemeMode, os_dark: bool) -> &str`. A selected scheme that does not exist
is a problem (lenient: falls back to the built-in of that mode).

**Open With (D-76).** `split_words(&str) -> Result<Vec<String>, ExpandError>`; `expand_command(template,
&ExpandContext) -> Result<Vec<String>, ExpandError>` where `ExpandContext { first: ExpandItem, second:
Option<ExpandItem>, line: Option<u64>, working_folder: PathBuf, path_delimiter: Option<char> }` and
`ExpandItem { path: PathBuf, base: PathBuf }` (absolute). `%x` of a name without extension is `""`; `%b` of
`.bashrc` is `.bashrc`; `%F`/`%P` are relative paths computed lexically (no file-system access); `%l` without a
line is `1`; the delimiter replaces both `/` and `\` in substituted paths only. `InstanceMode::of(template)` =
`PerPair` if any `%<v>2` occurs, else `PerItem`. A table test with ≥ 25 rows covers every variable, both
suffixes, quoting, `%%`, the errors and the Windows-style delimiter.

#### 1.1.5 Settings keys and Service calls (D-73)

```rust
pub enum FileId { Options, State, Commands, Theme, Themes(String), Filters, OpenWith, Archives }
pub struct SettingsKey { pub file: FileId, pub path: Vec<KeySegment> }   // KeySegment::{Name(String), Index(usize)}
pub fn parse_key(text: &str) -> Result<SettingsKey, ApiError>;          // invalid_input on bad syntax
pub enum ValueKind { Bool, Int { min: i64, max: i64 }, Float { min: f64, max: f64 }, Enum(&'static [&'static str]),
                     String, Path, Mask, Colour, Shortcut, List, Table }
pub struct KeyInfo { pub key: String, pub kind: ValueKind, pub default: toml::Value, pub opt: Option<&'static str>,
                     pub priority: Option<&'static str> }
pub fn key_infos(file: &FileId) -> Vec<KeyInfo>;    // every leaf key of the file's schema (from T::default())
```

`Service` gains (all read the configuration fresh; no caching across calls):

| Call | Behaviour |
|---|---|
| `settings_get(&SettingsKey) -> Result<SettingsValueDto, ApiError>` | effective value (file or default) of a leaf or subtree; unknown path → `not_found` with up to five suggestions (edit distance ≤ 2 over `key_infos`) |
| `settings_set(&SettingsKey, value_text: &str, force: bool) -> Result<SettingsSetDto, ApiError>` | parses `value_text` as a TOML value; if that fails and the key's kind is String/Path/Mask/Colour/Shortcut/Enum, the raw text is the string; type/range/validate errors → `invalid_input`; lenient load, then save (`Overwrite::IfUnchanged` unless `force`); setting a value equal to the default removes the key (sparse) |
| `settings_unset(&SettingsKey)` | removes the key (back to default); a kept unknown key, or a whole kept view or command, leaves the file (§1.4.20); same DTO |
| `settings_list(prefix: Option<&SettingsKey>, changed_only: bool) -> SettingsListDto` | every known leaf (plus unknown kept keys, flagged) in file order then key order; lenient load; warnings and shortcut conflicts attached |
| `settings_restore_defaults(files: &[FileId], dry_run: bool) -> Result<SettingsRestoreDto, ApiError>` | D-77: the plan (each file: `path`, `exists`) and, unless dry run, deletion; `FileId::Themes` with no name = every `themes/*.toml` |
| `format_new(id, from: Option<&str>, name: Option<&str>, mask: Option<&str>, kind: Option<FormatKind>)` | D-75: `formats/<id>.toml` from a copy of `from` (clone) or an empty format of `kind` (default text); id taken (built-in or user) → `already_exists`; appended to `order.toml` (created from the current order when absent) as enabled |
| `format_edit(id, set: &[(String, String)] or file: Option<&Path>)` | replaces the format with the parsed `file` (its `id` must equal `id`), or applies dotted `key=value` edits to the canonical TOML; the result must parse and compile, else `invalid_input` and nothing is written; editing a built-in writes its override file |
| `format_delete(id)` | user-only format: file removed, `order.toml` entry removed; built-in → `invalid_input` ("disable it instead"); unknown → `not_found` |
| `format_import(path, replace: bool)` | validates and copies to `formats/<id>.toml`; existing id without `replace` → `already_exists` |
| `format_export(id, out: Option<&Path>)` | canonical TOML to `out` (atomic) or returned for stdout |
| `format_enable(id, bool)`, `format_move(id, to: MoveTo)` | write `order.toml` only (`MoveTo::{Up, Down, Top, Bottom, Before(id), After(id)}`); first write creates `order.toml` from the current registry order |
| `format_restore(ids: &[String] or all)` | removes user override files of built-ins (and, with all, every user file and `order.toml`); a plan with `dry_run` like D-77 |

Every write call uses `write_atomic` and returns the touched paths. Formats keep their M1 parser and
canonical writer (`parse_format`, `to_toml`); `FormatOrder` gains the mutation helpers needed above.

#### 1.1.6 CLI and JSON

```
hikasa settings get <KEY>
hikasa settings set <KEY> <VALUE> [--force]
hikasa settings unset <KEY>
hikasa settings list [PREFIX] [--changed]
hikasa settings restore-defaults (--all | --file <FILE>…) (--dry-run | --yes)
hikasa format new <ID> [--from <ID>] [--name <TEXT>] [--mask <MASK>] [--kind <KIND>]
hikasa format edit <ID> (--from-file <PATH> | --set <KEY=VALUE>…)
hikasa format delete <ID>
hikasa format import <PATH> [--replace]
hikasa format export <ID> [-o <PATH>]
hikasa format enable <ID> | format disable <ID>
hikasa format move <ID> (--up | --down | --top | --bottom | --before <ID> | --after <ID>)
hikasa format restore (<ID>… | --all) [--dry-run | --yes]
```

Exit 0 on success, 100 on any error (kinds as in the table above; a usage error stays 100 with
`hikasa.usage_error.v1`). Human output: `get` prints the TOML value (a subtree as TOML), `set`/`unset` print
`<key> = <value>` and `(file unchanged)` when nothing was written, `list` prints `key = value` lines with
`  # default: …` when changed, `restore-defaults` and `format restore` print one line per file
(`delete <path>` / `absent <path>`), format writes print `wrote <path>`/`removed <path>`.

New schemas (appended to `SCHEMAS`, 19 → 26, each with a golden):

| Schema | `data` |
|---|---|
| `hikasa.settings_get.v1` | `{key, file, path, value, default, is_default, kind, opt}` (value/default as JSON; a subtree as an object) |
| `hikasa.settings_set.v1` | `{key, file, path, old, new, written}` (`unset` uses the same schema) |
| `hikasa.settings_list.v1` | `{entries: [{key, value, default, is_default, kind, opt, unknown}], conflicts: [{view, shortcut, commands}]}`; warnings in the envelope |
| `hikasa.settings_restore.v1` | `{dry_run, files: [{file, path, existed, deleted}]}` |
| `hikasa.format_write.v1` | `{action, id, written: [path], removed: [path]}` (new, edit, delete, import, enable, disable, move) |
| `hikasa.format_export.v1` | `{id, path or null, toml}` |
| `hikasa.format_restore.v1` | `{dry_run, ids, removed: [path]}` |

TOML values in JSON: string, integer, float, boolean as JSON scalars; arrays and tables as JSON arrays and
objects; datetimes (unused) as strings.

#### 1.1.7 Documentation updates of W1

`docs/03` §5.2 (new: `state.toml`, writing rules D-70, change detection D-74, `settings` restore D-77), §5.1
(D-31 → D-71 note); `docs/10` §1 (format write commands, D-75); `docs/12` §1 and the command table (`settings`,
the new `format` forms; `export|import` stay W2); `docs/14` §2.1 (new: theme and scheme files, D-78); `docs/15`
"Keys" (written with this contract) and the M1 section's pointer to D-71; `docs/16` M2 progress;
`help/settings.md` (new), `help/format.md`, `help/index.md` link; `CHANGELOG.md`.

#### 1.1.8 W1 notes (implementation)

*W1 note:* where the implementation had to interpret or extend the text above:

- **Order of kept keys.** `toml` is built without `preserve_order` (features are fixed at workspace level), so a
  table read from a file is sorted: kept unknown keys are written in sorted order after the known keys of their
  table, and unknown sections in sorted order after the known ones (save step 3). Known keys keep the declaration
  order of the schema (taken from its serde serialization).
- **Change stamps without a clock.** Core code may not read the clock (T1), so `ConfigWatch::poll` computes the
  CRC whenever length and time are unchanged (instead of only for files modified less than 2 s before the
  previous poll); settings files are small. `ConfigWatch::track_folder` (additive) is how `formats/` and `themes/`
  are listed at each poll.
- **Conflicts in `settings list`.** The catalogue binds the Copy and Take commands of the file views to the same
  keys by design (20 pairs), so `settings_list` reports `CommandOverrides::user_conflicts`: the conflicts that
  involve a command whose `shortcuts` the file overrides. `conflicts(view)` still returns all (W4 editor). In human
  mode they are `warning:` lines on stderr; there is no warning code for them.
- **Sparse entries.** Entries of maps and arrays of tables are also written sparsely against the defaults of a new
  entry (`[view.<v>]`, `[[app]]`, `[[preset]]`). A scheme file is sparse against the built-in of its base on every
  save (`SettingsSchema::save_defaults`; additive, default `T::default()`), whatever produced the value: a change
  of `base` keeps only the elements the file set, and colours compare as colours (`#FFFFFF` = `#ffffff`). Every
  W1 schema reads `schema_version = 0` as 1 (an identity migration; an M1 test loads such a file).
- **Arrays of tables in lenient loading.** Lenient loading never drops or reorders the elements of an array, so
  the positions in memory (`openwith.app.1`) are the positions in the file. A leaf of the wrong type inside an
  element is located by its full key (`app.1.wait_for_previous`) and repaired to the default of that leaf, like a
  scalar. A check that fails for a whole element (an empty required field, a duplicate preset name) names the
  element (`app.1`): the element is kept as it is, with warning `invalid_config_value` "(the entry is kept as it
  is)", and written back unchanged. `settings set` refuses only a problem the change adds, so the other entries
  (and the bad entry itself) stay editable.
- **Known keys.** A kind rule whose last segment is a wildcard (`tabs.*`) gives the kind of the keys a table has;
  it does not make another name a key, so a misspelt key is `not_found` with suggestions (D-73) and an unknown
  key kept in the file is shown by `get`/`list` as `unknown`. An array position past the end, a view that is not
  a view type and a command id that is not in the sections of its view are `not_found` for `get`, `set` and
  `unset` (`commands.view.home.command.NoSuch.menu` included).
- **Change stamps** of a load and of a later poll share one spelling of the modification time (a time before
  1970 is a negative number of nanoseconds).
- **API shapes.** `SettingsSchema::repair` has a default (on the serialized tree); `FILE` of the scheme schema is
  `FileId::Themes(String::new())`. Additive: `SettingsFile::{new_default, loaded, unknown, path}`,
  `SettingsStore::{file, scheme_file, resolved_scheme, scheme_names, scheme_files, dir}`, `FileId::{every, parse,
  relative_path, name}`, `FolderFont` (the folder font has `use_system`), `MruList`, `ServiceInit::options_mode`
  (`LoadMode::Lenient` for the `settings` commands; the service reads `options.toml` at construction),
  `ApiError::{NotFound, AlreadyExists}`, `Service::settings_store`, `parse_format_kind`. `store.scheme` returns
  `Loaded<SchemeFile>`; `settings_list` returns a `Result` (a newer or unparsable file is an error);
  `format_edit(id, set, file)` and `format_restore(ids, all, dry_run)` take both alternatives; `format_new --from`
  accepts an id or a name and names the copy `<name> (copy)`. The DTOs carry the load warnings (and the TOML
  value for human output) in fields the JSON skips.
- **Format ids.** `order` (in any case) is reserved: `format new`, `import` and `edit` refuse it with
  `invalid_input`, since `formats/order.toml` is the order file. `format new <id>` and `format import` without
  `--replace` are `already_exists` also when `formats/<id>.toml` exists but was skipped at load (it does not
  parse). `format new` writes `<id>.toml`, then `order.toml`; when the second write fails the first file is
  removed. The clap usage of `settings restore-defaults` and `format restore` names both alternatives
  (`<--all|--file <FILE>>`, `<ID|--all>`).
- **Human `format restore`** prints `delete <path>` for removed files and `absent <path>` for named ids without a
  file; `format edit --set` refuses unknown keys (the canonical writer would drop them); `--from-file` copies the
  file as it is (like `import`).

### 1.2 W2 — Sessions, workspaces, settings packages

**Ids.** SES-001…006, SES-008, SES-010, SES-021, SES-040…042, SES-060…062, ARCH-040, ARCH-041, ARCH-091
(`LoadSession`, `LoadWorkspace`), CLI-002 (session, workspace and package forms), CLI-G02 (`gui session`,
`gui workspace`), TC-052 (parent link data). Decisions D-80…D-91 (D-90 and D-91 added during the wave).

**Files.** `core/hikasa-session/src/sessions/` (`mod.rs`, `name.rs`, `file.rs`, `store.rs`, `autosave.rs`,
`scope.rs`, `close.rs`), `src/workspaces.rs`, `src/package/` (`mod.rs`, `manifest.rs`, `export.rs`,
`import.rs`), `src/api/sessions.rs`, `src/api/package.rs`; `cli/hikasa-cli/src/cmd/session.rs`,
`cmd/workspace.rs`, `cmd/settings.rs` (export/import/restore categories), `cmd/gui.rs` (`session`,
`workspace`); the legacy parser in `api/` (D-88); `gui/hikasa-gui` IPC handler only (`ipc.rs`, opening views
for loaded sessions with the existing view constructors); `help/session.md`, `help/workspace.md`,
`help/settings.md` (extended), goldens.

#### 1.2.1 Session file (D-80, D-81)

```toml
schema_version = 1
type = "folder-compare"        # folder-compare | folder-sync | text-compare | text-edit | table-compare |
                               # hex-compare | picture-compare
description = ""
locked = false
favorite = false               # SES-031 "Favorite" (pinned to the top of the tree)
saved_at = ""                  # RFC 3339; auto-saved files only
[parent]                       # TC-052: child sessions only
session = ""                   # reference of a named session, or "_autosaved/<id>"
left = ""  ;  right = ""       # the child's paths relative to the parent's bases ("/"-separated)
[specs]                        # canonical spec strings (absolute, `Spec` display form)
left = ""  ;  right = ""       # text-edit: `left` only
format = ""                    # file types: format id, "" = detect
left_encoding = ""  ;  right_encoding = ""
[rules]                        # folder-compare: FolderRules TOML (as --rules-file); file types: TextRules;
                               # others: opaque until W6/W12
[text_rules]                   # folder-compare and folder-sync: rules of rules-based content tests
[display]                      # opaque (SES-010)
[view]                         # opaque (layout, view state)
[[override]]                   # folder types only (D-82)
scope = "pair"                 # pair | type
left = ""  ;  right = ""       # pair: relative paths; type: unused
type = "text-compare"          # the child session type the override applies to
[override.rules]               # sparse against defaults(type)
```

`[rules]` and `[[override]].rules` are written sparse against the defaults of the type **as resolved at
save time** (D-82): a session saved before a change of the defaults keeps only its own differences. Specs of
`-` (stdin) and `clipboard://` are never stored (the session keeps `""` for that side and warning
`spec_not_saved`).

*Revised by D-90:* a save also keeps **every rule key the file already held** (with its value now), so an
explicit choice that happens to equal the defaults stays written and a later change of the defaults does not
change it; a key the file did not hold is written only when it differs from the defaults. `session edit --unset
<key>` removes a key. Nested unknown rule keys are kept. The optional folder rules (`criteria.timestamps.offset`,
`filters.other.size`, `filters.other.date`) are written whole, or `false` for "off" over defaults that set
them:

```toml
[rules.filters.other]
size = false                   # off, whatever the defaults say (also in --rules-file)
```

```rust
pub enum SessionType { FolderCompare, FolderSync, TextCompare, TextEdit, TableCompare, HexCompare, PictureCompare }
// as_str()/parse() = the kebab-case names above; parse of a P1 type name → ApiError::Unsupported
pub struct SessionName(String);                 // validated (D-80); Display = the reference
pub struct SessionRef { pub folder: Vec<SessionName>, pub name: SessionName }   // parse("a/b/c")
pub struct Session { pub kind: SessionType, pub description: String, pub locked: bool, pub favorite: bool,
                     pub saved_at: Option<jiff::Timestamp>, pub parent: Option<ParentLink>, pub specs: Specs,
                     pub rules: SessionRules, pub text_rules: Option<TextRules>, pub display: toml::Table,
                     pub view: toml::Table, pub overrides: Vec<Override> }
pub enum SessionRules { Folder(FolderRules), Text(TextRules), Opaque(toml::Table) }
impl Session {
    pub fn new(kind: SessionType, defaults: &Session) -> Session;      // SES-001: template = defaults
    pub fn clear(&self, defaults: &Session) -> Session;                // SES-008 Clear Session (keeps type)
    pub fn swap_sides(&mut self);                                      // SES-008: specs, encodings, parent paths,
                                                                       // pair overrides; `view` keys ending in
                                                                       // `_left`/`_right` are exchanged
    pub fn for_script(&self) -> Session;                               // SES-010: `display` cleared
}
```

#### 1.2.2 Store (`SessionStore`)

```rust
impl SessionStore {
    pub fn new(&ConfigLocation, &Options) -> SessionStore;
    pub fn list(&self) -> Result<SessionTree, SessionError>;          // folders, named, defaults, auto-saved (+ groups)
    pub fn load(&self, &SessionRef, LoadMode) -> Result<Loaded<Session>, SessionError>;
    pub fn defaults(&self, SessionType) -> Result<Session, SessionError>;   // missing file → built-in defaults
    pub fn save(&self, &SessionRef, &Session, Overwrite) -> Result<PathBuf, SessionError>;  // refuses locked unless Overwrite::Force
    pub fn save_defaults(&self, &Session) -> Result<PathBuf, SessionError>;
    pub fn delete(&self, &SessionRef) -> Result<(), SessionError>;
    pub fn rename(&self, from: &SessionRef, to: &SessionRef) -> Result<(), SessionError>;   // also moves between folders
    pub fn create_folder(&self, &[SessionName]) / delete_folder(&self, &[SessionName], recursive: bool)
        / rename_folder(&self, from: &[SessionName], to: &[SessionName]);
    pub fn autosave(&self, slot: Option<&AutoId>, &Session, now: &jiff::Zoned) -> Result<Option<AutoId>, SessionError>;
    pub fn prune_autosaved(&self, max: u32) -> Result<Vec<AutoId>, SessionError>;
    pub fn find(&self, text: &str) -> Option<SessionRef>;              // D-88: exact, then case-insensitive
}
pub enum AutoGroup { Today, Yesterday, Days2To6, Older }
pub fn auto_group(saved: jiff::civil::Date, today: jiff::civil::Date) -> AutoGroup;
```

Collisions: `save` to an existing reference with a different case of the same name → `already_exists`
(never two files differing only by case); `rename` onto an existing session → `already_exists`;
`delete_folder` of a non-empty folder without `recursive` → `invalid_input`. Every write is atomic; a
rename is `std::fs::rename` within the configuration folder (folders created on demand).

#### 1.2.3 Scope resolution (D-82)

```rust
pub enum Scope { View, Defaults, Pair, Type }
pub fn effective_rules(defaults: &Session, parent: Option<&Session>, child: &Session) -> SessionRules;
pub fn apply_scope(scope: Scope, change: &SessionRules, view: &mut Session, parent: Option<&mut Session>,
                   defaults: &mut Session) -> Result<ScopeWrites, ApiError>;  // which files to save
```

`Pair`/`Type` without a parent, or on a non-folder parent → `invalid_input`. A `pair` override replaces an
existing override for the same `(left, right, type)`; a `type` override for the same `type`. Table test with
every scope × (named parent, auto-saved parent, no parent) × (locked parent or not: a locked parent → the
override is kept in memory only, `ScopeWrites` says so).

#### 1.2.4 Close prompts (D-84)

`origin ∈ {Unnamed, AutoSaved, Named}`, `dirty` = specs or rules (not `view`/`display`) differ from the
loaded file, `explicit_save` = the user chose Save.

| origin | locked | dirty | explicit_save | `CloseAction` |
|---|---|---|---|---|
| Unnamed / AutoSaved | — | any | — | `AutoSave` (unless `max_auto_saved_sessions = 0`: `Nothing`) |
| Named | no | no | — | `SaveViewState` (only `view`/`display`, silently) |
| Named | no | yes | — | `AskSaveChanges` ("Save changes to session?": Yes → save, No → nothing, Cancel → stay) |
| Named | yes | any | no | `Nothing` |
| Named | yes | any | yes | `AskSaveLocked` ("Save changes to locked session?") |

#### 1.2.5 Workspaces (D-85)

```toml
schema_version = 1
shortcut = ""                  # SES-040 Manage Workspaces
[[window]]
width = 1200  ;  height = 800  ;  x = -1  ;  y = -1  ;  maximized = false   # as state.toml [window]
active_tab = 0
[[window.tab]]
session = "work/api-v2"        # a named session reference, or …
[window.tab.inline]            # … an inline session table (§1.2.1 without schema_version)
[window.tab.view]              # opaque per-view layout
```

`WorkspaceStore` (`list`, `load`, `save`, `delete`, `rename`) as the session store; exactly one of `session`
or `inline` per tab (problem otherwise); `active_tab` within range (lenient: 0). The GUI's "Save Workspace
As" and "On exit, save workspace as" call `Service::workspace_save` (W4).

#### 1.2.6 Packages (D-86, D-87)

```toml
# manifest.toml
schema_version = 1
format = "hikasa-pkg"
hikasa_version = "0.2.0"
created = "2026-10-28T12:00:00+01:00"          # injected clock
[[item]]
category = "sessions"                          # options | theme | commands | state | sessions | formats |
name = "work/api-v2"                           # workspaces | system_info
path = "sessions/work/api-v2.toml"
```

```rust
pub enum Category { Options, Theme, Commands, State, Sessions, Formats, Workspaces, SystemInfo }
pub struct ExportRequest { pub categories: Vec<Category>, pub items: BTreeMap<Category, Vec<String>> /* empty = all */ }
pub fn export(&ConfigLocation, &ExportRequest, now: &jiff::Zoned, env: &EnvFacts, out: &Path) -> Result<ExportDto, ApiError>;
pub struct ImportChoices { pub categories: Vec<Category>, pub items: BTreeMap<Category, Vec<String>>,
                           pub delete_existing: BTreeSet<Category> /* sessions, formats, workspaces only */ }
pub fn plan_import(&ConfigLocation, package: &Path, &ImportChoices) -> Result<ImportPlan, ApiError>;
pub fn execute_import(&ConfigLocation, &ImportPlan) -> ImportReport;            // per-item results
pub struct ImportPlan { pub items: Vec<PlanItem /* category, name, target, action: Add|Replace */>,
                        pub deletes: Vec<PathBuf>, pub summary: ImportSummary /* per category: added, replaced, deleted */ }
```

Export writes the zip atomically (temp + rename); entries are sorted by path, with a fixed modification time
(the manifest's `created`) so two exports of the same configuration at the same instant are byte-identical.
`system-info.txt`: Hikasa version, platform, configuration mode and folder, and the item counts — no user
names, no environment values. Import validation failures are `invalid_data` naming the entry; a newer
schema is `unsupported`; nothing is written. Execution order: deletes, then items in manifest order; a
failure of one item is recorded and the others continue (exit 100 when any failed). Restore defaults (D-87):
`plan_restore(categories, delete_sessions, delete_workspaces) -> ImportPlan` with only deletes.

#### 1.2.7 Service, CLI and JSON

```
hikasa session list [--folder <REF>] [--autosaved] [--defaults]
hikasa session show <REF> | session show --defaults <TYPE>
hikasa session new <REF> --type <TYPE> [--left <SPEC>] [--right <SPEC>] [--description <TEXT>]
hikasa session edit <REF> (--set <KEY=VALUE>… | --unset <KEY>… | --left <SPEC> | --right <SPEC>
                       | --description <TEXT>) [--force]
hikasa session edit --defaults <TYPE> (--set <KEY=VALUE>… | --unset <KEY>…)
hikasa session delete <REF> [--force] | session rename <REF> <NEW REF> [--force]
hikasa session lock <REF> | session unlock <REF>
hikasa session mkdir <FOLDER> | session rmdir <FOLDER> [--recursive]
hikasa session run <REF> [compare output flags of `folder compare` / `text compare`]
hikasa workspace list | workspace show <NAME> | workspace delete <NAME> | workspace rename <NAME> <NEW>
hikasa settings export -o <FILE.hikasa-pkg> [--category <CAT>…] [--item <CAT>:<NAME>…]
hikasa settings import <FILE> [--category <CAT>…] [--item <CAT>:<NAME>…] [--delete-existing <CAT>…]
                       [--force] (--dry-run | --yes | --silent)
hikasa settings restore-defaults (--all | --file <FILE>… | --category <CAT>…) [--delete-sessions [--force]]
                       [--delete-workspaces] (--dry-run | --yes)
hikasa gui session <REF> [--wait]    hikasa gui workspace <NAME>
```

`session edit --set` keys are paths inside the session file (`rules.ignore_case=true`), parsed and validated
like `settings set` (D-73), refused on a locked session unless `--force`; `--unset <key>` removes a key from the
file (D-90; it may be combined with `--set` of other keys). `session delete` and `session rename` of a locked
session exit 100 `invalid_input` unless `--force` (D-91). `session run` loads the effective
session (D-82) and runs the same Service call as `folder compare` / `text compare`: same exit codes, same
`hikasa.folder_compare.v1` / `hikasa.text_compare.v1` output (T11); other types → exit 100 `unsupported`
(Folder Sync after W6). `settings import --silent` = every category and item, no deletes, no prompt. A locked named session is kept by
`settings import` (replace and `--delete-existing sessions`) and by `restore-defaults --delete-sessions`: plan
action `skip_locked` with warning `session_locked`; `--force` includes it (D-91).

New schemas (appended to `SCHEMAS`): `hikasa.session_list.v1` (`{folders, sessions: [{ref, type,
description, locked, favorite}], defaults: [type], autosaved: [{id, type, saved_at, group, label}]}`),
`hikasa.session_show.v1` (`{ref, path, session}` — the session as JSON of its TOML), `hikasa.session_write.v1`
(`{action, ref, written: [path], removed: [path]}`), `hikasa.workspace_list.v1`, `hikasa.workspace_show.v1`,
`hikasa.workspace_write.v1`, `hikasa.settings_export.v1` (`{path, items: [{category, name, path}]}`),
`hikasa.settings_import.v1` (`{dry_run, plan: {items: [{category, name, path, action: add|replace|skip_locked}],
deletes, summary}, results: [{category, name, ok, error}]}`). `settings restore-defaults --category` keeps `hikasa.settings_restore.v1` with the plan's paths.
`gui session` / `gui workspace` reuse `hikasa.gui_open.v1`. Human output: one line per item, as in W1.

#### 1.2.8 Documentation updates of W2

`docs/03` §1 (D-80…D-84 interpretations), new §5.3 (session, workspace and package files), §6 (package
format, D-86/D-87); `docs/01` §3.3 (scope order) and §6 (IPC additions, D-89); `docs/12` §1 (CLI-002
resolution order, D-88; the new subcommands and `settings export|import`); `docs/02` (`zip` line);
`docs/16` progress; `help/session.md`, `help/workspace.md`, `help/settings.md`, `help/index.md`;
`CHANGELOG.md`.

#### 1.2.9 W2 notes (implementation)

*W2 note:* where the implementation had to interpret or extend the text above (the first four were accepted by
the maintainer during the wave):

- **Signatures.** `SessionStore` and `WorkspaceStore` methods return `Result<_, ApiError>` (they report
  `not_found`, `already_exists` and `invalid_input`, which `SessionError` has no variants for);
  `SessionStore::list(&self, now: &jiff::Zoned)` takes the injected clock's time for the groups (T1);
  `close_action(origin, locked, dirty, explicit_save, auto_save: bool)` takes whether auto-save is on (row 1 of
  §1.2.4 depends on `max_auto_saved_sessions`), with `is_dirty(loaded, current)` next to it. `TextCompareRequest`
  gains `base_rules: Option<TextRules>` and `FolderCompareRequest` `base_rules: Option<FolderRules>` and
  `base_text_rules: Option<TextRules>`: the rules a missing rules file means (`None`: the defaults), so
  `session run` and the GUI's `LoadSession` run the same Service calls as `folder compare` / `text compare` (T11).
- **Additive types.** `FileId::Session` and `FileId::Workspace` (engine-only files: no settings key names them;
  `hikasa settings` refuses them); `ApiError::Unsupported` (kind `unsupported`) and `ApiError::InvalidData`
  (kind `invalid_data`); warning codes `spec_not_saved` and `session_missing` in `hikasa_core::WarningCode`;
  `GuiOpenDto::views` (`gui workspace`: every opened view, omitted when empty). In memory `Override::rules` stays
  the sparse table of the file; `Session::saved_at` is a `Timestamp` (an auto-save writes it with the clock's
  offset, any other save in UTC).
- **Store.** `save(…, Overwrite)` has no stamp check (it takes a `Session`, not a loaded file): `Force` means "also
  over a locked file"; an existing file keeps its unknown keys. A folder that exists in another case is reused
  with its spelling. `Session::new` copies the type's defaults (description and specs included) without lock,
  favorite, parent or overrides. Auto-save pruning reads `saved_at` from each file (a file without a readable
  time counts as the oldest), ties broken by id. Listing skips unreadable files with warning
  `invalid_config_value`. Auto-saved groups are `today`, `yesterday`, `days_2_to_6`, `older` in JSON; the label
  is the description, else the input names (`a ⇄ b`).
- **`session` commands.** `session list` always lists folders and named sessions; `--autosaved` and `--defaults`
  add their sections (the JSON keys are always present); `--folder` restricts to a folder (case-insensitive).
  `session show` prints the file as it is (the defaults without a file: the built-in defaults' file).
  `session edit --set` may change `description`, `favorite`, `parent`, `specs`, `rules`, `text_rules`, `display`
  and `view` (the defaults: `description`, `rules`, `text_rules`, `display`, `view`; `--left`/`--right` are
  refused there); a key must exist in the whole session with every rule spelled out (else `not_found` with up to
  five suggestions), except below the opaque `display` and `view`; text rules must compile. `session run` takes
  the union of the output flags of both compare commands (`--report` defaults to `unified` or `list` by type); a
  failure before the type is known uses `hikasa.folder_compare.v1` (exit 100). The plan's `session export|import`
  are settings packages (`settings export --item sessions:<ref>`), not `session` subcommands.
- **Packages.** Export without `--category`/`--item` takes every category but `system_info`; import without them
  takes every category of the package; `system_info` named for import is `invalid_input`. Item names:
  `options`/`filters`/`openwith`/`archives`, `theme`, `themes/<scheme>`, `commands`, `state`, a session
  reference or `_defaults/<type>`, a format id or `order`, a workspace name, `system-info`. `system-info.txt`
  writes the configuration folder with the home folder as `~`. Zip entries are deflated (level 6 through
  `flate2`/`zlib-rs`; `zopfli` is compiled in by the `deflate` feature but unused at that level), Unix mode
  0644, the manifest's `created` local time. An item whose target is deleted first is `add`; `--delete-existing
  sessions` deletes named sessions and `_defaults` (never `_autosaved`). A failed delete is a result whose name is
  its path. Every package entry (also of categories not chosen) is validated before anything is written.
  `restore-defaults --category` keeps `hikasa.settings_restore.v1`: `file` is the category, `existed` is true (only
  existing files are planned); `--delete-sessions` and `--delete-workspaces` require `--category` and add their
  category.
- **IPC and GUI (D-89).** The server passes `LoadSession` (with a waiter when `wait`) and `LoadWorkspace` to the
  handler. The GUI handler answers an unknown name `not_found`, a type it cannot show `unsupported_request`, any
  other failure `invalid_request`; `LoadWorkspace` first closes the open views (SES-040), skips Home tabs and
  logs the warnings of tabs it cannot open. Sessions open with `display` cleared (as a script loads them,
  SES-010). The CLI checks names and types locally before contacting the GUI. `docs/01` §6 of §1.2.8 is §5.1
  there (the IPC section); the additions are written in §5.1.
- **D-88.** The one-argument forms are resolved by `Service::classify_single_arg` for `hikasa gui open`; the
  window's own command line (`hikasa-gui <name>`) keeps the M1 forms until W4. With `--silent`, a package is
  imported as `settings import --silent` and prints `hikasa.settings_import.v1` (the switch's `switch_ignored`
  warning is dropped); without it, exit 100 `unsupported` names the wizard of W4.
- **Review fixes (D-90, D-91).** Import validation is strict: each session file is read with
  `LoadMode::Strict` and its rules, text rules and every `[[override]]` are built and checked
  (`FolderRules::validate`, `TextRules::validate`), regular expressions compiled with `RegexLimits::default()`
  (`plan_import` has no `Service`; these are the limits the registry uses, and no code changes them today —
  revisit if they become configurable); the inline sessions of workspace tabs are checked the same way. A
  package with two entries whose paths differ only by case is `invalid_data`; an item's target is matched
  case-insensitively against existing files and folders and against earlier items of the plan (the existing
  spelling is kept), in every category. An unknown manifest category is `invalid_data`. `--item` without
  `--category` exports the categories of its items only. `session edit --set specs.*` stores the spec like
  `--left`/`--right`. `session edit --unset` (also with `--defaults`) takes keys below `rules`, `text_rules`,
  `display` and `view`; an unknown key is `not_found`. Removing an entry of a map the defaults hold (an
  `importance.elements` entry) is not representable (D-90): deferred to W4. Locked sessions (D-91): new warning
  code `session_locked` in `hikasa_core::WarningCode`; `ImportChoices::force`, `plan_restore(…, force)` and
  `Service::settings_restore_categories(…, force, dry_run)`; a `skip_locked` item has no entry in `results`
  (import) or in `files` (restore) and is not counted in the summary — the warning names it.
  `SettingsImportDto::warnings` and `SettingsRestoreDto::warnings` carry the plan's warnings (not serialized in
  `result`; the envelope's `warnings` holds them). An auto-save whose slot holds a locked file writes nothing
  and keeps the slot. `CONIN$`, `CONOUT$` and the superscript device names (`COM¹`…`COM³`, `LPT¹`…`LPT³`) are reserved names
  too.

### 1.3 W3 — VFS mutations, clipboard, generic copy

**Ids.** ARCH-031 (generic cross-back-end copy), ARCH-030 and VFS-001 (the new capabilities; operations degrade by
capability), the clipboard back-end (`docs/11` §1 table, P0, no id), TC-002 (the clipboard part), TC-065 (data:
the Service call that reads the clipboard), FC-073 (recycle-bin primitive), FC-076 (set-times primitive), FC-077
(create-folder primitive; archive extensions stay W5), ARCH-101 (per-item results of `vfs` write commands),
CLI-000 (the new subcommands comply). Decisions D-92…D-103. No archive back-end (W5), no folder operation plans,
backups or confirmation model (W6): §1.3.10 names the hooks they build on.

**Files.** `core/hikasa-core/src/` (`error.rs`: `ErrorKind::NotEmpty`; `progress.rs`: `Phase::Writing`; `job.rs`:
`JobKind::VfsWrite`; `spec/mod.rs`: `Spec::parent`); `core/hikasa-vfs/src/` (`vfs.rs`, `caps.rs`, `error.rs`,
`router.rs`, `memory.rs`, `local/mod.rs`, `local/unix.rs`, `local/windows.rs`, new `local/trash.rs`, new `ops/`
with `mod.rs`, `plan.rs`, `copy.rs`, `remove.rs`, `rename.rs`, `touch.rs`, `lib.rs`); `core/hikasa-vfs/tests/`
(`mutations.rs`, `ops.rs`, `ops_faults.rs`, `symlinks.rs`, `trash.rs`); `core/hikasa-session/src/clipboard.rs`
(new), `src/host.rs` (`SystemClipboard`, `FileClipboard`), `src/api/mod.rs` (`ServiceInit::clipboard`, the blob
router), new `src/api/vfs_ops.rs` (requests, DTOs), `src/api/dto.rs`, `src/api/human.rs`, `src/api/gui.rs`
(D-101), `src/ipc/protocol.rs` (doc comment of `OpenView::specs`); `cli/hikasa-cli/src/args.rs`,
`cmd/vfs.rs`, `cmd/gui.rs`, `main.rs` (provider wiring); `gui/hikasa-gui` provider wiring and IPC handler only
(`main.rs`, `ipc.rs`; D-101); workspace `Cargo.toml`, `core/hikasa-vfs/Cargo.toml`,
`core/hikasa-session/Cargo.toml`, `deny.toml` (only if a new transitive licence appears: stop and ask),
`core/hikasa-session/tests/architecture.rs` (manifest table and two confinement rules, §1.3.9); `help/vfs.md`,
`help/cli.md`, `help/gui.md`; goldens.

#### 1.3.1 Mutating primitives (D-92, D-94, D-99)

The `Vfs` trait gains **single-item** primitives. Recursive and cross-back-end algorithms are not trait methods:
they live once in `hikasa_vfs::ops` (§1.3.4) and call these primitives, so `MemoryVfs` faults test the
algorithms of every back-end. Every new method has a default body that returns
`VfsError::OperationUnsupported { spec, op }`, so existing implementors compile unchanged.

```rust
// hikasa_vfs::vfs (additive)
/// What `set_times` changes; `None` keeps the current value. The access time is never changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetTimes { pub modified: Option<FileTime>, pub created: Option<FileTime> }

/// What `set_attrs` changes. Owner and group are never changed (D-99).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrChange {
    /// Unix permission bits: `mode & 0o7777` is applied; other bits of the argument are an `InvalidAttributes`.
    Mode(u32),
    /// DOS attributes: `set` are turned on, `clear` turned off, the others kept. `set ∩ clear ≠ ∅` →
    /// `InvalidAttributes`.
    Dos { set: DosAttrs, clear: DosAttrs },
}

pub struct WriteOptions {
    pub create_parents: bool,
    pub overwrite: bool,
    /// New (breaks struct literals; use `..Default::default()`). `false` (default, M1): a target that is a
    /// symbolic link is resolved and the file it points to is replaced (save semantics, `docs/11` §1 "Local
    /// writes"). `true`: the link itself is replaced by the new file and the file it pointed to is untouched
    /// (copy semantics, D-94).
    pub replace_link: bool,
}

pub trait Vfs: Send + Sync {
    // … the M1 methods unchanged …

    /// Creates the folder `spec`. `parents = false`: a missing parent is `NotFound`, an existing item of any kind
    /// is `AlreadyExists`. `parents = true`: missing ancestors are created; an existing folder (or a link to one)
    /// is success; an existing non-folder (at `spec` or an ancestor) is `NotADirectory`. `Ok(true)` when `spec`
    /// was created, `Ok(false)` when it already was a folder.
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError>;
    /// Removes one item: a file, a special file, a symbolic link (the link itself, never its target; on Windows
    /// also a folder link or junction) or an **empty** folder. A non-empty folder is `NotEmpty`; a missing item
    /// `NotFound`.
    fn remove(&self, spec: &Spec) -> Result<(), VfsError>;
    /// Renames within this back-end (atomic where the platform renames atomically). The final components of
    /// `from` and `to` are never followed (a link is renamed, or replaced, as a link). `replace = false`: an
    /// existing `to` is `AlreadyExists`. `replace = true`: a file or link replaces a file or link; a folder
    /// replaces only an empty folder (else `NotEmpty`); folder ↔ non-folder is `IsADirectory`/`NotADirectory`.
    /// A rename whose `to` is the same item as `from` (case-only rename on a case-insensitive back-end; checked by
    /// file identity) changes the spelling and always succeeds. Different file systems → `CrossDevice`.
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError>;
    /// Sets the times of the item itself; a symbolic link gets its own times where the platform allows (else
    /// `OperationUnsupported`). `created` needs `SET_CREATED`, else `OperationUnsupported` (nothing changed).
    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError>;
    /// Changes the attributes of the item itself. On a symbolic link → `OperationUnsupported` (Unix has no
    /// portable `lchmod`; a link's own attributes are never what the user means). The wrong family (`Mode` on a
    /// DOS back-end, `Dos` on a Unix one) → `OperationUnsupported`.
    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError>;
    /// Creates a symbolic link at `spec` whose target text is `target`, stored verbatim (never resolved or
    /// rewritten). `target_is_dir` makes a folder link on Windows and is ignored elsewhere. Existing item →
    /// `AlreadyExists`; no privilege (Windows) → `PermissionDenied`.
    fn create_symlink(&self, spec: &Spec, target: &OsStr, target_is_dir: bool) -> Result<(), VfsError>;
    /// Moves one item (a file, a link as a link, or a whole folder) to the platform's recycle bin. Never falls
    /// back to a permanent delete: any failure is an error and the item stays (D-98).
    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError>;
    /// A back-end-specific copy of the regular file `from` to `to` (both on this back-end) with exactly the
    /// semantics of `open_write(to, opts)` + streaming + `commit`: atomic, target untouched on failure or
    /// cancellation. `Ok(None)`: no fast path for these specs (the caller streams). Default: `Ok(None)`.
    fn copy_file_fast(&self, from: &Spec, to: &Spec, opts: &WriteOptions, cancel: &CancelToken)
        -> Result<Option<u64>, VfsError> { Ok(None) }
}
```

`Spec::parent(&self) -> Option<Spec>` (hikasa-core, additive): the parent of a local path (`None` for a root),
`None` for every other spec kind in W3 (W5 adds archive members). The parent folders of every operation are
reached through `Spec::parent` and `Vfs::child`; nothing joins path strings.

**Capabilities (VFS-001, ARCH-030).** `SET_TIMES` (bit 6, unused in M1) now means `set_times(modified)` works.
New bits, appended in this order (names in JSON, §4.1 rule 9; `NAMES` grows from 14 to 21):

| Bit | Flag | Name | Meaning |
|---|---|---|---|
| 14 | `MKDIR` | `mkdir` | `create_dir` |
| 15 | `REMOVE` | `remove` | `remove` |
| 16 | `RENAME` | `rename` | `rename` |
| 17 | `SET_ATTRS` | `set_attrs` | `set_attrs` for the family the back-end reports (`DOS_ATTRS` or `UNIX_MODE`) |
| 18 | `TRASH` | `trash` | `move_to_trash` |
| 19 | `CREATE_SYMLINKS` | `create_symlinks` | `create_symlink` |
| 20 | `SET_CREATED` | `set_created` | `set_times` with `created` |

| Back-end | Capabilities added in W3 |
|---|---|
| `LocalVfs`, Linux and other Unix | `SET_TIMES MKDIR REMOVE RENAME SET_ATTRS TRASH CREATE_SYMLINKS` |
| `LocalVfs`, macOS | the Unix set plus `SET_CREATED` |
| `LocalVfs`, Windows | `SET_TIMES MKDIR REMOVE RENAME SET_ATTRS TRASH CREATE_SYMLINKS SET_CREATED` (`SET_ATTRS`: the `R` bit only, D-99; `CREATE_SYMLINKS` may still fail with `PermissionDenied`) |
| `MemoryVfs` | all 21 (as before, `set_capabilities` removes some; a primitive whose bit is removed returns `OperationUnsupported`) |
| blobs (`-`, `clipboard://`) | none (unchanged: `READ SEEK MTIME`); every primitive → `OperationUnsupported` |

Capabilities stay platform sets (M1): a mount that refuses an operation (a read-only file system, a network share
without a recycle bin) fails at the call with the OS error, per item.

**Router dispatch** (extends the M1 table of `router.rs`):

| Spec | `create_dir`, `remove`, `set_times`, `set_attrs`, `create_symlink`, `move_to_trash` | `rename(from, to)` | `copy_file_fast` |
|---|---|---|---|
| `Local` | `LocalVfs` | `LocalVfs` when both are `Local` | `LocalVfs` when both are `Local` |
| `Stdin`, `Clipboard` | `OperationUnsupported` | `OperationUnsupported` | `Ok(None)` |
| archive, snapshot, remote, profile, HTTP | `Unsupported { what: "<scheme> specs" }` (D-05) | same | `Ok(None)` |
| mixed kinds | — | `OperationUnsupported` (callers fall back to copy + remove only on `CrossDevice`, D-97) | `Ok(None)` |

**`LocalVfs` implementation notes (normative where stated).** `create_dir`: `fs::create_dir` /
`fs::create_dir_all`. `remove`: `fs::symlink_metadata` first; a link → `fs::remove_file` (Unix) or, on Windows,
`fs::remove_dir` for a folder link or junction and `fs::remove_file` otherwise; a folder → `fs::remove_dir`
(`io::ErrorKind::DirectoryNotEmpty` → `NotEmpty`); anything else → `fs::remove_file`. `rename`: `lstat(to)` for
the `replace = false` check and the same-item check (`same-file` handles of both, case-only renames), then
`fs::rename` (`io::ErrorKind::CrossesDevices` → `CrossDevice`). The check-then-rename is not atomic against a
racing writer (no `renameat2` without `unsafe`); the window is documented in `docs/11`. `set_times`: `filetime`
(`set_symlink_file_times` for links, keeping the current access time); `created` through
`std::fs::File::set_times` with the platform's `FileTimesExt::set_created` (Windows, macOS; a folder is opened
with backup semantics on Windows). `set_attrs`: Unix `fs::set_permissions` with `PermissionsExt::from_mode`
(never on a link: `lstat` first); Windows `Permissions::set_readonly` for `R`, any other bit in `set`/`clear` →
`Unsupported { what: "changing DOS attributes other than R" }`. `create_symlink`: `std::os::unix::fs::symlink`,
`std::os::windows::fs::{symlink_file, symlink_dir}`. `move_to_trash`: `local/trash.rs` calls `trash::delete` on
the absolute path (the crate canonicalizes the parent only, so a link is trashed as a link on the same file system; across file systems the crate would copy what the link points to, so such a link is refused, §1.3.12; D-98).
`copy_file_fast`: §1.3.4 step 5.

#### 1.3.2 Errors (D-93)

New `VfsError` variants (the enum is `#[non_exhaustive]`) and the new `ErrorKind::NotEmpty` (`"not_empty"`,
hikasa-core, additive; `ErrorKind` is `#[non_exhaustive]`):

| Variant | `kind()` | Message | `From<VfsError> for io::Error` |
|---|---|---|---|
| `NotEmpty { spec }` | `not_empty` | `{spec}: folder is not empty` | `DirectoryNotEmpty` |
| `CrossDevice { from, to }` | `unsupported` | `{from} -> {to}: cannot rename across file systems` | `CrossesDevices` |
| `OperationUnsupported { spec, op: VfsOp }` | `unsupported` | `{spec}: {op} is not supported here` (op as `VfsOp::as_str`) | `Unsupported` |
| `InvalidTarget { spec, reason: &'static str }` | `invalid_input` | `{spec}: {reason}` | `InvalidInput` |
| `Unavailable { spec, reason: String }` | `unsupported` | `{spec}: {reason}` | `Unsupported` |

`InvalidTarget` reasons (stable text): `"is the source itself"`, `"is inside the source folder"`, `"is not an
existing folder"`, `"needs a file name"`, `"is a root folder"`, `"changed since the plan was made"`, `"changed
during the move; the copy was kept"` (§1.3.12).
`Unavailable` is the clipboard's "no clipboard" (§1.3.6). `VfsError::spec()` returns `from` for `CrossDevice`
and the spec for the others. `VfsError::from_io` adds `DirectoryNotEmpty` → `NotEmpty` and `CrossesDevices` →
`CrossDevice { from: spec, to: spec }` (callers that know both specs build it themselves);
`hikasa_core::io_error_kind` adds `DirectoryNotEmpty` → `NotEmpty`. `VfsOp` gains `Mkdir`, `Remove`, `Rename`,
`SetTimes`, `SetAttrs`, `Symlink`, `Trash` (serde and `as_str`: `mkdir`, `remove`, `rename`, `set_times`,
`set_attrs`, `symlink`, `trash`; `VfsOp::ALL` grows to 14). Exit codes are unchanged (§2.21 of M1): every `vfs`
error and every failed item is 100; `ApiError::exit_code` needs no new row (`ErrorKind::NotEmpty` falls into
"everything else").

#### 1.3.3 Symbolic links (D-94)

Normative, for every back-end with `SYMLINKS` (a table test per row, §6.3 A3.6):

| # | Operation | Rule |
|---|---|---|
| 1 | `remove`, `move_to_trash`, `rename`, `set_times` | act on the link itself; the target is never touched |
| 2 | `set_attrs` on a link | `OperationUnsupported` |
| 3 | `ops::remove_tree` | lists with `FollowLinks::No`; a link inside the tree is removed as a link; a link to a folder is never descended. Before listing a folder it re-`lstat`s it and requires `kind == Dir` and, where both are known, the `FileId` seen in the parent's listing (narrows, but cannot close, the race with a process that swaps a folder for a link; documented) |
| 4 | copy/move whose **target** item is a link | the link is replaced (`WriteOptions::replace_link = true`, `rename(…, replace = true)`); the file it pointed to is unchanged |
| 5 | copy/move into a target **subfolder** that is a link | never written through: the link is a non-folder target item, so the source folder meets a kind conflict (D-96: `Fail` → `already_exists`, `Skip` → skipped subtree; `Always` replaces the link by a real folder, then copies into it) |
| 6 | copy of a link (source) | `LinkPolicy::Copy` (default): a new link with the same target text (`create_symlink`, `target_is_dir` from following the source link; a broken link copies as a link to a file); `LinkPolicy::Skip`: skipped, reason `symlink`. A target back-end without `CREATE_SYMLINKS` → skipped, reason `symlinks_unsupported`, warning `capability_missing`. Links are never followed inside a tree |
| 7 | root arguments that are links | taken as links (rule 6) unless `follow_roots` (CLI `--follow-symlinks`): then a root link is replaced by what it points to, for that root only |
| 8 | intermediate components (`/data` → `/mnt/data`) | followed, as by every OS call; only the final component of an operated item is ever protected |
| 9 | `open_write` without `replace_link` (save, `--output`) | unchanged M1 behaviour: the file the link points to is replaced |

Copying links as links (rather than following them) is what keeps a copied tree faithful, never pulls data from
outside the source tree into the target, and cannot loop; following stays a per-root, explicit choice. W6 may add
a "follow links inside trees" policy with loop detection if Folder Compare's follow option needs it.

#### 1.3.4 Generic copy (ARCH-031; D-95, D-96)

```rust
// hikasa_vfs::ops
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum OverwritePolicy { #[default] Fail, Skip, Always, Newer }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum LinkPolicy { #[default] Copy, Skip }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preserve { pub times: bool, pub attrs: bool, pub created: bool }   // Default: times and attrs, not created
pub struct CopyOptions { pub recursive: bool, pub overwrite: OverwritePolicy, pub links: LinkPolicy,
                         pub preserve: Preserve, pub follow_roots: bool, pub create_parents: bool }
pub struct MoveOptions { pub overwrite: OverwritePolicy, pub create_parents: bool }
```

**Overwrite policy (D-96).** Decided per item when the target exists (`lstat`); a link at the target counts as a
file (D-94 rule 4):

| Source \ target | absent | file or link | folder |
|---|---|---|---|
| file or link | copy | `Fail` → item fails `already_exists`; `Skip` → skipped `exists`; `Always` → replace; `Newer` → replace when the source `mtime` is strictly newer, else skipped `not_newer` (an unknown `mtime` on either side → skipped `not_newer`) | fails `is_a_directory` (whatever the policy: a folder is never deleted to make room for a file) |
| folder | create, then its contents | `Always` → the file or link is removed, then the folder created; other policies as for files | merged: the folder is kept (`keep_dir`), its contents follow the policy item by item |
| special file (FIFO, socket, device) | skipped `special_file` (never opened: reading a FIFO can block forever) | same | same |

**Algorithm of one file** (normative; `src` and `dst` are `(vfs, spec)` pairs, possibly different back-ends):

1. `lstat` the source (`FollowLinks::No`; `FollowLinks::Yes` for a root with `follow_roots`). Kind dispatch as in
   the table; a link → D-94 rule 6.
2. Same-item and nesting checks (planning, §1.3.5): `dst == src` (same back-end and same canonical spec, or the
   same `FileId`) → `InvalidTarget "is the source itself"`; `dst` lexically inside a source folder (canonical
   specs, case-folded when the back-end is not `CASE_SENSITIVE`) → `InvalidTarget "is inside the source folder"`.
3. `lstat` the target; apply the overwrite table.
4. If `src.vfs` and `dst.vfs` are the same instance (`std::ptr::addr_eq`), call `copy_file_fast`; `Ok(Some(n))`
   → step 6.
5. Otherwise stream: `w = dst.open_write(dst, WriteOptions { create_parents, overwrite: <decided in step 3>,
   replace_link: true })`, `r = src.open_read(src)`, copy in buffers of `StreamOptions::effective_buffer_size`,
   checking `ctl.cancel` before every buffer and reporting `Phase::Writing` bytes; any read, write or cancel error
   drops `w` (the temporary sibling is removed, the target is untouched: INV-VFS-3) and fails the item; then
   `w.commit()`. **`LocalVfs::copy_file_fast`** (local → local): files up to `FAST_COPY_MAX = 64 MiB` are copied
   with one `std::io::copy` from the source into the open handle of a temporary sibling created exactly as
   `open_write` creates one (never reopened by name, so a read-only source works; `copy_file_range` on Linux),
   then committed with the same rename protocol; larger files return `Ok(None)` (streamed), because that call cannot be
   cancelled and G4 (cancel ≤ 100 ms) holds for copies too. The OS copy may carry more metadata (Windows: `H`,
   `S`, `A` and times; macOS: extended attributes); that is allowed, the steps below run anyway.
6. Metadata, in this order, each only when its `Preserve` field is set: **times** (`set_times { modified: src.mtime,
   created: src.ctime if preserve.created and SET_CREATED }`), then **attributes** (`set_attrs` with the
   source's mode or DOS bits; between families nothing is mapped). Times before attributes because a read-only
   result must not stop the time change; owner and group are never copied (D-99). A folder's metadata is applied
   **after its whole subtree** (creating children changes a folder's `mtime`). A missing capability on the target
   (or no `mtime` on the source) skips that step silently and adds **one** warning `capability_missing` per
   operation naming the capability; a failing call of a supported step fails the item with that error and **keeps**
   the copied content (the item reports `failed`, the file is complete).
7. Never rolled back: a committed file stays even if a later item fails.

**Bytes and progress.** `Phase::Scanning` (`Unit::Items`) while planning, `Phase::Writing` (`Unit::Bytes`, total =
the plan's bytes) while executing; `current` = the item's target spec.

#### 1.3.5 Plans, execution and per-item results (D-103, T12)

Every `vfs` write operation is computed as a complete **plan** before the first write, and executed from that
plan; `--dry-run` prints the same plan and writes nothing (T12). The plan holds every item of every tree (the
whole source tree is listed during planning, so a target inside a source can never be copied into itself, and
the plan is finite).

```rust
#[derive(Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum OpKind { Copy, Move, Remove, Mkdir, Touch }
#[derive(Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Action { CreateDir, KeepDir, Copy, Link, Rename, Move, Remove, RemoveDir, Trash, Touch, Create, Skip }
#[derive(Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum SkipReason { Exists, NotNewer, SpecialFile, Symlink, SymlinksUnsupported, ChildrenFailed }
#[derive(Serialize, Deserialize)]
pub struct PlanItem { pub action: Action, pub kind: EntryKind, pub source: Option<Spec>, pub target: Spec,
                      pub size: Option<u64>, pub replaces: bool, pub skip: Option<SkipReason>,
                      pub error: Option<ErrorReport>, pub depth: u32 }
#[derive(Serialize, Deserialize)]
pub enum PlanOptions { Copy(CopyOptions), Move(MoveOptions), Remove(RemoveOptions), Mkdir { parents: bool },
                       Touch(TouchOptions) }
#[derive(Serialize, Deserialize)]
pub struct Plan { pub op: OpKind, pub options: PlanOptions, pub items: Vec<PlanItem>, pub warnings: Vec<Warning> }
#[derive(Serialize)] #[serde(rename_all = "snake_case")]
pub enum ItemStatus { Planned, Done, Skipped, Failed, NotRun }
pub struct ItemResult { pub status: ItemStatus, pub skip: Option<SkipReason>, pub error: Option<ErrorReport> }
pub struct Summary { pub planned: u64, pub done: u64, pub skipped: u64, pub failed: u64, pub bytes: u64 }
pub struct OpReport { pub plan: Plan, pub results: Vec<ItemResult>, pub summary: Summary, pub cancelled: bool,
                      pub warnings: Vec<Warning> }
impl OpReport { pub fn ok(&self) -> bool; }          // failed == 0 && !cancelled

pub fn resolve_targets(dst: &dyn Vfs, sources: &[Spec], target: &Spec, into_folder: bool, create_parents: bool)
    -> Result<Vec<(Spec, Spec)>, VfsError>;
pub fn plan_copy(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &CopyOptions, ctl: &Ctl<'_>)
    -> Result<Plan, VfsError>;                       // Err only for Cancelled
pub fn plan_move(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &MoveOptions, ctl: &Ctl<'_>)
    -> Result<Plan, VfsError>;
pub fn plan_remove(vfs: &dyn Vfs, specs: &[Spec], opts: &RemoveOptions, ctl: &Ctl<'_>) -> Result<Plan, VfsError>;
pub fn plan_mkdir(vfs: &dyn Vfs, specs: &[Spec], parents: bool) -> Plan;
pub fn plan_touch(vfs: &dyn Vfs, specs: &[Spec], opts: &TouchOptions) -> Plan;
pub fn dry_run(plan: Plan) -> OpReport;              // every result Planned (or Failed for items with an error)
pub fn execute(src: &dyn Vfs, dst: &dyn Vfs, plan: Plan, ctl: &Ctl<'_>) -> OpReport;
```

**`resolve_targets`** (the `cp`/`mv` naming rule, owned by core so the CLI, scripts and the GUI agree, T11):
`target` an existing folder (a root link to a folder is followed: the user named it) → each source goes to
`target/<source file name>`; otherwise one source → `target` is the new name, several sources → `InvalidTarget
"is not an existing folder"`; `into_folder` (the argument ended with a path separator) and `target` missing →
with `create_parents` the folder is planned (`create_dir`), else `NotFound`. A source without a file name (`-`,
`clipboard://`) into a folder → `InvalidTarget "needs a file name"`.

**Plan rules.** Items are in **execution order**: roots in argument order; copy and move pre-order (a folder's
`create_dir`/`keep_dir`, then its children sorted by `Listing::sort_by_name`); remove post-order (children, then
the folder). A listing failure or a per-entry error is an item with `error` set (its subtree is not planned);
a root that cannot be read is an item with `error` set; other roots continue (ARCH-101). Planning never writes.
`depth` is the item's level below its root (0 = root).

**Execution rules.** Items run in order. Each item re-checks its target with `lstat`: an item planned without a
target (`create_dir`, `copy`, `link`, `rename`, `move`, `create`) that now finds one fails with `InvalidTarget
"changed since the plan was made"` — execution never replaces anything the plan did not show; an item planned to
replace that finds no target just writes. An item with `error` from planning is `failed` without a call. A folder
whose child failed or was skipped is not removed by `remove`/`move` (status `skipped`, reason
`children_failed`). Below a folder item of a copy or move that failed or was skipped, and below a folder that
changed since the plan was made, nothing runs: those items are `not_run` (§1.3.12); `newer` is decided again.
Cancellation is checked between items and inside copies: the running item fails with
`cancelled` (its target untouched), later items are `not_run`, `cancelled = true`. Exit code: 0 when `ok()`, else
100 (also when cancelled).

**Move (D-97).** Per root: same back-end instance, same spec scheme, `RENAME` on both sides and the target absent
(or replaceable by the policy) → one `rename` item; but when both `FileId::dev` values are known (the source and
the target's existing parent) and differ, planning expands the root at once. Otherwise — another back-end, or a
target folder that exists (merge) — the root is **expanded**: `create_dir`/`keep_dir` for folders, `move` (=
copy + remove source) for files and links, then `remove_dir` of each source folder after its contents. A
`rename` that fails with `CrossDevice` at execution is expanded there and executed in place; the report's plan
shows the expansion (the dry run could not know it; `docs/11` says so). A `move` item whose source removal fails
removes the new copy again (rollback) when it did not replace an existing target, and fails with the removal's
error; when it replaced one, the copy stays and the item fails; when the source is no longer the item copied
(gone, or another item in its place), the copy stays and the item fails (§1.3.12).

**Remove and trash (D-98).** `RemoveOptions { recursive: bool, trash: bool }`. Without `recursive` a non-empty
folder is an item error `NotEmpty` (planning lists the folder once); an empty folder is removed. With `trash`
each root is one `trash` item (`move_to_trash` of the whole item, never per child); a non-empty folder still
needs `recursive`. A root that is a file-system root (`Spec::parent() == None`) → `InvalidTarget "is a root
folder"`, for `remove` and for `move` sources.

**Touch and mkdir.** `TouchOptions { times: SetTimes, create: bool }`: an existing item → `touch`
(`set_times`); a missing item → `create` (an empty file through `open_write`, then `set_times`) with `create`,
else an item error `NotFound`. `plan_mkdir`: a missing folder → `create_dir`; an existing folder → `keep_dir`
with `parents`, else item error `AlreadyExists`.

#### 1.3.6 Clipboard (D-100, D-101)

```rust
// hikasa_session::clipboard (pure)
pub enum ClipboardContent { Text(String), Image { width: u32, height: u32, rgba: Vec<u8> } }
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("no clipboard is available ({0})")] Unavailable(String),   // headless, no display, not supported
    #[error("the clipboard holds no text or image")] Empty,
    #[error("cannot read the clipboard: {0}")] Failed(String),
    #[error("the clipboard content is larger than {0} bytes")] TooLarge(u64),   // W3 review, §1.3.12
}
pub trait ClipboardProvider: Send + Sync { fn read(&self) -> Result<ClipboardContent, ClipboardError>; }
pub struct NoClipboard;                                   // Unavailable("no clipboard provider"); ServiceInit::new's default
pub struct FixedClipboard { /* content or error */ }     // tests: FixedClipboard::text(..), ::image(..), ::empty(), ::unavailable(..); counts reads
pub fn clipboard_blob(content: ClipboardContent, mtime: FileTime) -> Result<Blob, VfsError>;
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, VfsError>;

// hikasa_session::host (adapter-only, like SystemClock)
pub struct SystemClipboard;      // arboard: get_text, else get_image, else Empty
pub struct FileClipboard(pub PathBuf);   // the file's bytes as text: missing → Unavailable, empty → Empty
pub fn clipboard_provider(env: &EnvFacts) -> Arc<dyn ClipboardProvider>;   // FileClipboard when
                                 // env.clipboard_file_override is set, else SystemClipboard
```

- **Content.** Text wins when the clipboard holds both. Text → the UTF-8 bytes, no name hint (format detection by
  content, as for `-` without `--stdin-name`). Image → a PNG (`encode_png_rgba`: 8-bit RGBA, colour type 6, one
  IDAT of stored deflate blocks, `crc32fast` CRCs, Adler-32; deterministic; > 1 GiB of pixels → `TooLarge`), name
  hint `clipboard.png`. The PNG bytes are not a stable format beyond "a PNG that decodes to the clipboard's pixels"
  (W12 may switch to a compressing encoder). Blob `mtime` = the injected clock at capture (T1).
- **Capture.** `ServiceInit` gains `clipboard: Arc<dyn ClipboardProvider>` (breaks literals; `ServiceInit::new`
  sets `NoClipboard`). The provider is called **only** when a `clipboard://` spec is parsed (`Service::parse_spec`)
  — never for other commands, so no display connection is opened by them — and **once per service**: both sides of
  `quick clipboard:// clipboard://` see the same snapshot. From then on the router serves the blob for
  `Spec::Clipboard` beside the stdin blob. `Service::read_clipboard(&self) -> Result<ClipboardDto, ApiError>`
  (TC-065 data, the GUI) reads the provider **again** and replaces the snapshot for later calls; `ClipboardDto {
  kind: text|image, size, name: Option<String> }`. `Service::vfs()` returns `Arc<VfsRouter>` (a snapshot; was
  `&VfsRouter`) because the router changes when a blob is captured.
- **Errors.** `Unavailable` → `VfsError::Unavailable { spec: "clipboard://", reason }` (kind `unsupported`);
  `Empty` → `VfsError::NotFound { spec: "clipboard://" }` (kind `not_found`); `Failed` → `VfsError::Io` (op
  `read`). In `quick`/`text compare` they arrive as `LoadFailed` (exit 100); in `vfs cat|stat|cp` as the
  command's error (exit 100). A failure is cached for the service like stdin's (`read_clipboard` retries).
- **Where `clipboard://` works.** `quick`, `text compare`, `vfs cat`, `vfs stat`, `vfs cp clipboard://
  <file>` (needs an explicit file name), and `gui open` / `--gui` (D-101). It stays refused as a rules file over
  IPC, is never stored in a session (D-80), cannot be written (`OperationUnsupported`) and is not a folder
  (`folder compare` → `NotADirectory`, M1).
- **`arboard`.** `SystemClipboard` creates an `arboard::Clipboard` per read. Error mapping: initialization failure,
  `ClipboardNotSupported` and `ClipboardOccupied` → `Unavailable(<arboard's message>)`; `ContentNotAvailable`
  for text and image → `Empty`; `ConversionFailure` and others → `Failed`. A slow selection owner is bounded by
  `arboard`'s own timeouts (no timer in core).
- **Tests never touch the real clipboard.** Unit and Service tests use `FixedClipboard`; CLI tests set
  `HIKASA_CLIPBOARD_FILE` (captured by `capture_env` into the new `EnvFacts::clipboard_file_override`, like
  `HIKASA_IPC_ENDPOINT`). Only `cli/` and `gui/` name `SystemClipboard` / `clipboard_provider` (architecture
  check, §1.3.9).

**`gui open clipboard://` (D-101, revises D-49).** The CLI sends `clipboard://` **verbatim** in `OpenView.specs`
(never reads its own clipboard for the GUI); `-` keeps the M1 spool. `Service::open_view_request` accepts
`clipboard://` (still refuses `-` and relative specs); the GUI's IPC handler calls `Service::read_clipboard`
before it opens the view, so the GUI reads **its own** clipboard (same desktop session as the user; the CLI may
run headless). A clipboard error is answered `Error{invalid_request}` with the message (exit 100 at the CLI). A
view loaded from `clipboard://` keeps the bytes it loaded; when a reload re-reads the clipboard is a W11 choice.
The Home view's refusal of a typed `clipboard://` stays until W4/W11 lift it.

#### 1.3.7 Service, CLI and JSON (D-102)

```rust
// hikasa_session::api::vfs_ops
pub enum VfsOpRequest {
    Copy   { sources: Vec<OsString>, target: OsString, options: CopyOptions },
    Move   { sources: Vec<OsString>, target: OsString, options: MoveOptions },
    Remove { specs: Vec<OsString>, options: RemoveOptions },
    Mkdir  { specs: Vec<OsString>, parents: bool },
    Touch  { specs: Vec<OsString>, time: TouchTime, created: Option<FileTime>, create: bool },
}
pub enum TouchTime { Now, At(FileTime), Reference(OsString) }   // Now = the injected clock; Reference = its mtime
impl Service {
    pub fn vfs_plan(&self, req: &VfsOpRequest, job: &JobControl) -> Result<VfsOpDto, ApiError>;  // --dry-run
    pub fn vfs_run(&self, req: &VfsOpRequest, job: &JobControl) -> Result<VfsOpDto, ApiError>;   // plan + execute
}
```

The trailing separator of the `cp`/`mv` target argument (`/`, and `\` on Windows) is read from the raw
`OsString` before parsing (`into_folder`). Both calls use the service's router for source and target. Whole-command
errors (`ApiError`: spec syntax, `resolve_targets`, usage) exit 100 with the failure envelope; item failures do not
make an `ApiError`.

```
hikasa vfs cp <SRC>… <DST> [-r|--recursive] [--overwrite fail|skip|always|newer] [--symlinks copy|skip]
                          [--follow-symlinks] [--no-preserve] [--preserve-created] [-p|--parents] [--dry-run]
hikasa vfs mv <SRC>… <DST> [--overwrite fail|skip|always|newer] [-p|--parents] [--dry-run]
hikasa vfs rm <SPEC>… [-r|--recursive] [--trash] (--dry-run | --yes)
hikasa vfs mkdir <SPEC>… [-p|--parents] [--dry-run]
hikasa vfs touch <SPEC>… [--time <RFC3339> | --reference <SPEC>] [--created <RFC3339>] [--create] [--dry-run]
```

- `cp` without `--recursive` and a folder source → item error `IsADirectory` ("use --recursive"). `--no-preserve`
  clears every `Preserve` field; `--preserve-created` sets `created`. `--follow-symlinks` = `follow_roots`.
- `mv` moves folders without `--recursive`. `rm` needs `--yes` unless `--dry-run` (D-77 rule): without either →
  exit 100 `invalid_input` "rm deletes items: pass --yes, or --dry-run to see the plan", and nothing is planned
  or touched. `cp`, `mv`, `mkdir` and `touch` need no `--yes`: they replace nothing unless `--overwrite
  always|newer` says so, which is the explicit choice.
- `touch` with neither `--time` nor `--reference` uses the injected clock's now; `--time` and `--reference`
  conflict (clap). `--created` needs `SET_CREATED` at the item (else that item fails `unsupported`).
- `JobKind::VfsWrite` (`"vfs_write"`) for every write command and its dry run.

**Human output** (stdout, one line per plan item in plan order, then the summary; names escaped with
`escape_controls`): the word of the action, then `<target>` (`mkdir`, `exists` for `keep_dir`, `rmdir`, `remove`,
`trash`, `touch`, `create`) or `<source> -> <target>` (`copy`, `replace`, `link`, `rename`, `move`); a skipped item
`skip (<reason>) <target>`; after execution a failed item `failed <word> <target>: <message>` and a not-run item
`not run <word> <target>`. Summary: `<verb> <done> of <planned> items (<skipped> skipped, <failed> failed)` with
`<verb>` = `copied`, `moved`, `removed`, `created`, `touched` (ARCH-101); a dry run ends with `would <verb>
<planned> items`, a cancelled run with `cancelled`. Warnings (`capability_missing`) on stderr as usual.

**JSON.** One new schema for the five commands (appended to `SCHEMAS`, 34 → 35):

| Schema | `result` |
|---|---|
| `hikasa.vfs_op.v1` | `{op: "copy"\|"move"\|"remove"\|"mkdir"\|"touch", dry_run: bool, cancelled: bool, items: [{action, kind, source: str\|null, target: str, size: u64\|null, replaces: bool, status: "planned"\|"done"\|"skipped"\|"failed"\|"not_run", reason: str\|null, error: ErrorReport\|null}], summary: {planned, done, skipped, failed, bytes}}` |

Item failures are not an envelope error: `ok: true`, `exit_code: 100`, `error: null`, the failures in `items`
(as `hikasa.settings_import.v1`, W2). The envelope's `warnings` hold the plan's and execution's warnings.
`hikasa.vfs_stat.v1` keeps its shape; its `capabilities` show the new names (goldens change).

#### 1.3.8 `MemoryVfs` (D-92)

`MemoryVfs` implements every primitive with POSIX semantics (as its M1 reads), honours `set_capabilities`, and
adds test hooks (additive):

| Hook | Effect |
|---|---|
| `Fault { op: Mkdir \| Remove \| Rename \| SetTimes \| SetAttrs \| Symlink \| Trash, .. }` | the primitive on that path fails with the kind (`Rename` matches the `from` path); nothing changes |
| `inject_read_error_after(path, bytes: u64, kind)` | `open_read(path)` succeeds; reads return the first `bytes`, then fail |
| `inject_write_error_after(path, bytes: u64, kind)` | `open_write(path)` succeeds; `write` fails once `bytes` were accepted (the writer is then dropped by the caller: nothing visible) |
| `Fault { op: Commit, .. }` (M1) | `commit` fails; nothing visible |
| `set_device(prefix: &str, dev: u64)` | items under `prefix` get `FileId { dev, .. }`; `rename` between different devices → `CrossDevice` |
| `set_journal(bool)`, `journal() -> Vec<(VfsOp, String)>` | every mutating call in order (`commit` as `Commit`), for order assertions (content → times → attributes; folder times after children) |
| `trashed() -> Vec<String>` | the original paths moved to the trash, in order (the items leave the tree) |
| `dump() -> Vec<MemDump>` | deterministic snapshot of the tree: path, kind, length and CRC-32 of content, mtime, ctime, attrs, link target (for "target untouched" assertions) |

`set_times` on a link and `create_symlink` are supported; `set_attrs` follows D-94 rule 2. Paths follow the host
syntax (M1); case-insensitive instances fold names on every primitive and a case-only `rename` changes the kept
spelling.

#### 1.3.9 Dependencies and architecture test

- **`trash`** 5 (MIT; 5.2.9 at the time of writing): `[workspace.dependencies] trash = { version = "5",
  default-features = false, features = ["chrono", "coinit_apartmentthreaded"] }` (`chrono` writes the
  `DeletionDate` the freedesktop trash requires; apartment-threaded COM is the crate's default on Windows), used by
  `hikasa-vfs` only, called only from `core/hikasa-vfs/src/local/trash.rs`. New transitive crates: `chrono` (Linux
  and the BSDs), `windows` 0.62 (Windows), `objc2-foundation` (macOS); licences MIT or MIT/Apache.
- **`arboard`** 3 (MIT OR Apache-2.0; 3.7.0 already in `Cargo.lock` through `egui-winit`): `arboard = { version =
  "3", default-features = false, features = ["image-data", "wayland-data-control"] }` (the feature set the lock
  already compiles; no new crate), used by `hikasa-session`, named only in `core/hikasa-session/src/host.rs`.
- `docs/02`: a line for each (version, features, crate, why), and the M1 constraint row on `arboard` replaced by
  "D-100, D-101". `cargo deny check` must pass without new exceptions.
- Architecture test: manifest table `hikasa-vfs` += `trash`, `hikasa-session` += `arboard`; check 2 gains two
  confinement rules: the token `trash::` only in `core/hikasa-vfs/src/local/trash.rs`; the token `arboard` only in
  `core/hikasa-session/src/host.rs` under `core/`. `core/hikasa-vfs/src/local/trash.rs` is an allow-point for
  `std::env` (the home trash folder of the link check, §1.3.12). `SystemClipboard` and `clipboard_provider` join `capture_env`
  and `SystemClock` as adapter-only names (referenced only by `cli/`, `gui/` and hikasa-session's own tests).
- T1: `trash` reads the home and data folders from the environment and the clock for `DeletionDate` inside the
  call. This is a side effect of the OS operation, not an input of a result, so it is accepted (D-98) and the
  call is confined to one file.

#### 1.3.10 Hooks for W5 and W6

- **W5 (archives).** `ArchiveVfs` implements the same primitives (and `open_write`) by staging changes and
  rebuilding the archive atomically (temp + rename, Q8). `ops::execute` is the single place an operation ends,
  so W5 may add a defaulted `Vfs::finish(&self, at: &Spec)` called there once per touched back-end root (one
  rebuild per operation) without changing the primitives; a failed rebuild turns that root's done items into
  failed ones. `Spec::parent` and `resolve_targets` gain archive members. `create_dir` of a name with an archive
  extension creates an empty archive (FC-077) in W5, a plain folder in W3.
- **W6 (folder operations, sync).** `OpPlan` wraps `ops::Plan` items with sides, conflicts (read-only, system,
  newer), confirmation answers and backups; `Action::Copy` with `replaces = true` and `Action::Remove` are the two
  points where a backup step and the recycle-bin choice (`RemoveOptions::trash`, FC-073 "if possible") are
  inserted. Per-item results, the execution re-check, cancellation and the move fallback are reused as they are.
  "Delete to recycle bin if possible" (FS-005) falling back to a permanent delete is a W6 confirmation choice;
  the W3 primitive never falls back.

#### 1.3.11 Documentation updates of W3

`docs/01` §3.2 (the trait sketch gains the mutating primitives; ARCH-031 points to `ops`); `docs/02` (the
`trash` and `arboard` lines; the M1 `arboard` constraint row); `docs/10` (format detection of clipboard content:
text without a name, images as `clipboard.png`); `docs/11` §1 (Clipboard row: text or image, captured once per
command, errors; "Local writes" gains the mutation rules, D-94 and the rename race; recycle bin, D-98); `docs/12`
§1 (the `vfs` row with the W3 forms; the D-49 note of §1.1 replaced by D-101) and §4 (flags table, human output
table, schema list with `hikasa.vfs_op.v1`, the `HIKASA_CLIPBOARD_FILE` variable); `docs/16` M2 progress;
`help/vfs.md` (read and write commands, plans, `--dry-run`/`--yes`, overwrite policies, links, trash,
`clipboard://`; `hikasa help vfs cp` resolves to it by the D-67 fallback), `help/cli.md` (specs: `clipboard://`
and its errors), `help/gui.md` (`gui open clipboard://`); `CHANGELOG.md`. The command catalogue does not change
(the GUI file-operation commands stay `planned` until W9), so `docs/reference/commands.md` is unchanged.

#### 1.3.12 W3 notes (implementation)

*W3 note:* where the implementation had to interpret or extend the text above (the maintainer's choices on the
open points were applied: `rm` needs `--yes` also with `--trash`; on Windows only `R` is settable; clipboard images
are the in-house uncompressed PNG until W12; the default overwrite policy is `fail`; `HIKASA_CLIPBOARD_FILE` is a
documented, user-visible override):

- **Errors.** `hikasa_core::io_error_kind` also maps `CrossesDevices` → `unsupported`, so `From<VfsError> for
  io::Error` keeps the kind of `CrossDevice` (the round trip of §1.3.2 holds for every variant). `fault_io` of
  `MemoryVfs` maps `ErrorKind::NotEmpty` → `DirectoryNotEmpty`. `AttrChange::validate` (additive) checks the
  argument (`Mode` above `0o7777`, overlapping `Dos` sets → `InvalidAttributes`).
- **Capabilities.** `MemoryVfs` reports 20 of the 21 bits: every W3 bit, but not `LOCAL_PATH` (it has no local
  paths, as in M1). `LocalVfs::move_to_trash` and the `MemoryVfs` primitives refuse a root with `InvalidTarget "is
  a root folder"`; `ops` refuses roots for **local** specs only (`Spec::parent` is `None` for blobs, which reach
  the primitive and report `OperationUnsupported`).
- **`replace_link`** also replaces a FIFO or device at the target by the new file (copy semantics) instead of
  writing into it, and never copies the permissions of a replaced link or special file. `MemoryVfs::open_write`
  without `replace_link` follows a link to the file it points to; onto a dangling link it creates the file the
  chain of links names (as `LocalVfs`, M1 save semantics; a missing folder there is `NotFound`).
  `MemoryVfs::create_dir(parents)` through a broken link is `NotADirectory` (as `LocalVfs`); `set_attrs` on a node
  without attributes creates the family the capabilities allow (`Mode` → uid/gid 0). The journal records
  successful calls only. `LocalVfs::rename`'s same-item check uses device + inode where known; without file ids
  (Windows) two links are the same item only when their paths differ by case alone.
- **Copy metadata.** Within the DOS family only the `R` bit is copied (D-99: other bits cannot be set on
  Windows); between families nothing is copied and no warning is given. Only **created** folders get the
  source folder's metadata (a merged `keep_dir` folder is kept as it is); links get no metadata. A missing
  `SET_CREATED` with `--preserve-created` is one `capability_missing` warning (`set_created`). In a move, a failing
  metadata step fails the item and keeps both the source and the copy (no data is lost).
- **Plans.** `plan_copy`, `plan_move` and `plan_remove` take the `Ctl` of the job (not a `CancelToken`): they
  report `Phase::Scanning` (`Unit::Items`, `done` = items planned so far, `current` = the spec being planned) and
  check cancellation per item, so planning a huge tree can be cancelled (`Err(Cancelled)`, nothing written);
  `Phase::Writing` is reported while executing. `plan_mkdir`/`plan_touch` plan only the named items and keep
  their signatures. A dry run's skip items are `planned` with their `reason` (as "every result Planned");
  human output prints them as `skip (<reason>) <target>`. Items failed at planning carry `size: null`. A folder
  given to `cp` without `-r` is a `create_dir` item with the error `is_a_directory` and the message "… is a folder
  (use --recursive)". `mkdir -p` of an existing link to a folder is `keep_dir`. Blocking a parent folder
  (`children_failed`) uses the item's source spec where it has one (moves), else its target; a failed
  `create_dir` of a move blocks the source folder's `remove_dir` too.
- **Human output.** A dry run ends with the base verb: `would copy|move|remove|create|touch <n> items` (the past
  participle of §1.3.7 reads wrongly after "would").
- **Clipboard capture.** `Service::parse_spec` never fails for `clipboard://`: a failed read is cached as
  `hikasa_vfs::BlobFailure` in the router (`VfsRouter::with_clipboard_failure`, additive), whose `stat` and
  `open_read` return the error of §1.3.6, so `quick`/`text compare` report `LoadFailed` and `vfs cat|stat|cp` the
  command's (or the item's) error, all exit 100. The specs a write command writes, removes or touches (and the
  `cp`/`mv` target) are parsed without reading standard input or the clipboard. Additive API:
  `clipboard::{CLIPBOARD_SPEC, CLIPBOARD_IMAGE_NAME, MAX_IMAGE_BYTES, ClipboardError::to_vfs,
  FixedClipboard::{failing, set_text, reads}}`, `api::ClipboardDto` (`kind` `"text"`/`"image"`). The PNG CRCs use
  `hikasa_vfs::crc32_reader` (no CRC crate in `hikasa-session`'s manifest). `FileClipboard` takes the file's bytes
  as UTF-8 with invalid sequences replaced. "Text wins over an image" lives in `SystemClipboard::read` and has no
  automated test (tests never touch the real clipboard).
- **GUI handover.** The handler logic "read the clipboard when a spec is `clipboard://`, then build the request"
  is the core call `Service::open_view_from_ipc` (additive, T11); the GUI handler only calls it and answers an
  error `invalid_request`. The Home view's refusal of a typed `clipboard://` is unchanged.
- **CLI.** `--overwrite` and `--symlinks` take free words parsed by core (`api::parse_overwrite`,
  `parse_link_policy`, T11); an unknown word is `invalid_input` (exit 100). `rm --dry-run --yes` is a usage error.
  `api::ends_with_separator` reads the trailing separator of the raw target.
- **`trash`.** Across devices the crate copies into the trash and deletes the original (still recoverable, not a
  permanent delete). For a **link** that copy follows the link (the trash would get a copy of what it points to,
  or a stray placeholder for a link to a folder), so on the freedesktop platforms `local/trash.rs` refuses a link
  whose device differs from the device of the home trash folder (`$XDG_DATA_HOME/Trash`, else
  `$HOME/.local/share/Trash`, or its nearest existing parent): `VfsError::Unavailable` (kind `unsupported`),
  nothing changed. It reads those two variables itself, as the crate does inside the call (D-98); the
  architecture test allows `std::env` in that one file. The check is conservative: such a link is refused even
  where a per-volume trash (`$topdir/.Trash-$uid`) would have kept it as a link; a bind mount of the same file
  system (same device, `rename` still `EXDEV`) is not detected. macOS and Windows trash through the OS, which
  keeps links. New transitive crates `chrono`, `iana-time-zone`, `urlencoding` (no new licences).
- **Review fixes (items and roots).** Every item an operation removes, moves (source and target), copies onto,
  trashes, creates or touches is refused when its final component is `.` or `..` (`InvalidTarget "needs a file
  name"`), and `rm`/`mv`/trash refuse a local path that is a root once `..` is resolved lexically (`"is a root
  folder"`). `LocalVfs::{remove, rename, move_to_trash}` check the same (and a root reached through the
  canonical parent). Unix keeps a final `..` in a spec but drops a final `.`, so `Service::vfs_plan`/`vfs_run`
  also check the raw `rm` items and `mv` sources (`api::is_dot_name`; `.`, `x/.`, `../`): a whole-command
  `invalid_input` before planning. `cp x .` still copies into the current folder (the item is `./x`).
- **Review fixes (replacing).** Replacing a file or link by a link (`link`, the `move` of a link) or by a folder
  (`create_dir` with `replaces`) renames the old item to a free hidden sibling `.hikasa-old[-n]`, creates
  the new one, then removes the sibling (best effort: a sibling that cannot be removed is left, never the
  data lost); when the creation fails the sibling is renamed back, so the target is
  as before (a back-end without `RENAME` removes first). The window in which the target is the sibling is
  accepted (a folder cannot be renamed over a file atomically); the fault hooks address the target path.
- **Review fixes (subtrees).** In copies and moves, every item below a `create_dir`/`keep_dir` that failed or
  was skipped (the following items deeper than it) is `not_run` — never attempted, so nothing is written through
  a link swapped in for a merged folder and no source of a move below it is removed; `not_run` blocks the
  parents' `remove_dir` (`children_failed`). Folder items carry the `FileId` seen at planning (`PlanItem::seen`,
  crate-private, not serialized: `hikasa.vfs_op.v1` and the plan's JSON are unchanged): before an item at depth
  > 0 runs, the folder holding it (on the source side: the source of copies and moves, the folder of removes)
  must still `lstat` as a folder with that `FileId` (where both are known; else every planned folder above must
  still be a folder). A changed folder makes everything below it `not_run`; its own `create_dir`/`keep_dir`
  (source re-check) or `remove_dir` fails "changed since the plan was made". A root followed by
  `--follow-symlinks` is checked through the link. The re-check narrows, but cannot close, the race (D-94 rule 3).
- **Review fixes (execution).** `--overwrite newer` is decided again for every replacing item: a target that is
  no longer older is skipped `not_newer`. A `move` whose source removal fails rolls the copy back only while the
  source still `lstat`s as the item copied (kind and `FileId`); otherwise the copy (now the only one) is kept and
  the item fails `InvalidTarget "changed during the move; the copy was kept"` (spec: the source). A case-only
  `mv` on a case-insensitive back-end (the target is the source spelt differently: the same comparison key or
  `FileId`, texts that differ only by case) is one `rename` to the literal target, also for folders
  (`resolve_targets` does not move a folder into itself; the execution re-check accepts the target as the
  source itself). With `preserve.times` a source without `mtime` adds one `capability_missing` warning per
  operation ("the source has no `mtime`").
- **Review fixes (blobs).** `resolve_targets` refuses a blob target (`-`, `clipboard://`) with
  `OperationUnsupported` (op `write`) before anything is read; `plan_remove`, `plan_mkdir`, `plan_touch` and
  `plan_move` (sources) give a blob item `OperationUnsupported` without calling the back-end, so the M1 router
  text "… is not supported in this version" no longer reaches the write commands.
- **Review fixes (clipboard).** Empty clipboard text is an empty clipboard (`not_found`; `SystemClipboard` then
  tries an image). `clipboard::MAX_TEXT_BYTES` = 1 GiB (the image limit; stdin has no limit to follow) caps
  clipboard text and `HIKASA_CLIPBOARD_FILE` (a larger regular file is refused from its length, others are read
  at most one byte past the limit); an image above `MAX_IMAGE_BYTES` is refused before its pixels are copied.
  Additive: `ClipboardError::TooLarge(limit)` and `BlobFailure::TooLarge(limit)` (kind `too_large`). The PNG
  encoder writes the stored deflate blocks straight into the output from the rows (peak memory: the pixels and
  the PNG, no intermediate copies).
- **Architecture test.** The confinement rule for `trash::` ignores module paths (`self::trash::`,
  `super::trash::`: `local/mod.rs` calls its own `trash` module); `SystemClipboard` is refused in every test file
  of `core/`, `cli/` and `gui/` (A3.9).

### 1.4 W4 — GUI shell: windows, tabs, menus, shortcuts, theming, options, Home, sessions

**Ids.** UI-001…008, UI-010, UI-020…023 (UI-023: the colours are editable; Picture Compare paints them in W13),
UI-030, UI-031, SES-001…006 (GUI), SES-020…022 (dialog shell), SES-030, SES-031, SES-040…042 (GUI), SES-053
(notice), SES-060…062 (wizards), SES-090, SES-091, SES-092 (framework), SES-093, FC-031 (configurable colours),
FC-100 (panel), CLI-002 (the GUI's own command line, D-88), OPT-001, OPT-004…009, OPT-011, OPT-012, OPT-014,
OPT-015, OPT-017…021 (the P0 keys of the Options dialog; each takes effect in the wave of its feature), ARCH-GUI-003
(re-measured), G12. Decisions D-104…D-134. Out of scope (W5+): archives, folder operations and sync, scripts, the
report and Quick Compare dialogs, text editing, the table/hex/picture views, the VFS browse dialog (W9), the rule
tabs of the Session Settings dialog (W9/W11/W13).

This section is written like M1 §1.10: it states the **GUI's needs** — the window model, the view-model rules, the
`Service` calls (§1.4.15, marked *exists* or *new*), theme application, the icon inventory and the notification
sources. Core keeps all logic (T11, D-105); the GUI holds presentation and the toolkit-free models that are tested
without a window (§1.4.16).

**Parts (D-104).** Sequential, one implementer each; each ends green with its acceptance subset of §6.4.

| Part | Content | Requirement ids | Acceptance |
|---|---|---|---|
| **W4a Shell** | §1.4.1–§1.4.8: windows and tabs, layout memory, commands from the catalogue and `commands.toml` (menus, toolbars and their right-click menu, tab context menu, shortcuts, hints), dialog framework and prompts, theme application, icons, status bar and jobs, message panel, log book and panel, notifications and polling, OS drag and drop | UI-001…008, UI-010, UI-020…022 (applied), UI-030, UI-031, SES-053, SES-090, SES-091, SES-092 (framework), FC-031, FC-100, OPT-004, OPT-005, OPT-015, OPT-017…019 (hints, MRU) | A4.1–A4.10 |
| **W4b Sessions** | §1.4.9–§1.4.12: sessions of views (save, lock, close prompts, auto-save), Session Settings dialog shell, Home view, workspaces, startup and exit, the GUI command line (sessions and workspaces) | SES-001…006, SES-020…022, SES-030, SES-031, SES-040…042, SES-093, CLI-002 (GUI), OPT-001, OPT-019 (auto-save limit) | A4.11–A4.16 |
| **W4c Settings dialogs** | §1.4.13–§1.4.14: Options dialog (every P0 page, the Toolbars/Shortcuts/Menus page, the theme editor with live preview), Export/Import/Restore wizards, IPC `ImportPackage` and `.hikasa-pkg` on the command line | UI-003 (editor), UI-020…023 (editor), OPT pages, SES-060…062, FC-031 (editor) | A4.17–A4.23 |

**Files.**

- *W4a.* `core/hikasa-session/catalogue/commands.toml` (the `toolbar` flags, the new tab entries, statuses);
  `core/hikasa-session/src/commands.rs` (`toolbar` field, `effective`, `ViewCommands`); new `src/log.rs`; new
  `src/watch.rs` (`InputStamp`, `InputState`, `InputWatch`); `src/settings/state.rs` (`layout`);
  `src/settings/mod.rs` (`SettingsSnapshot`, `SettingsChanges`); `src/api/settings.rs` (`settings_snapshot`,
  `settings_save`, `state_update`, `log_append`, `input_stamp`), `src/api/mod.rs` (`reload_settings`, the
  `options()`/`registry()` snapshots, D-119); `gui/hikasa-gui/src/`: `app.rs` (windows), new `window.rs` (window
  and tab operations), `commands.rs` (presentation table, `Invocation`), new `menus.rs` (menu and context-menu
  assembly), new `dialog.rs` (`DialogModel`, `DialogStack`, `Prompt`, About), new `status.rs` (status items, job
  list, message panel), new `notify.rs` (notices), new `watch.rs` (the `hikasa-watch` thread), new `dnd.rs`,
  `ui/mod.rs` (viewports), `ui/theme.rs` (`Palette` from `Scheme`), new `ui/icons.rs`, new `ui/menus.rs`,
  `ui/dialog.rs`, `ui/status.rs`, `ui/notify.rs`, `ui/log.rs`, `ui/keys.rs`, `folder_view.rs`/`text_view.rs`
  (status items, message panel, palette, `ShowLog`, reload on input change), `main.rs`, `bench.rs`
  (`two_windows`), `tests.rs` (or `src/tests/` split by area), `tests/perf_gate.rs`;
  `core/hikasa-session/tests/architecture.rs` (check 7: model files name no `egui`/`eframe`, §1.4.16);
  `help/main-window.md`.
- *W4b.* `core/hikasa-session/catalogue/commands.toml` (the `Tree*` entries of `home`, statuses);
  `core/hikasa-session/src/help.rs` (topic list); `src/api/sessions.rs` (the new session and workspace calls of
  §1.4.15; `view_of` made public as `session_view_of`); `gui/hikasa-gui/src/`: new `view_session.rs`
  (`ViewSession`, auto-save timer), `app.rs` (close flow, pending tabs), `home.rs` and `ui/home.rs` (rewritten: tree, search, management
  panel, drop targets), new `session_dialog.rs` + `ui/session_dialog.rs`, new `workspace.rs` +
  `ui/workspace.rs` (Save As, Manage), `ipc.rs` (`LoadSession`/`LoadWorkspace` through the window model), `main.rs`
  (startup workspace, D-88 forms), `bench.rs` (`workspace`), tests; help `view-home.md` (rewritten), new
  `dialog-session-settings.md`, `dialog-save-session.md`, `dialog-workspaces.md`.
- *W4c.* `core/hikasa-session/catalogue/commands.toml` (statuses); `core/hikasa-session/src/help.rs` (topic
  list); `src/settings/commands.rs` (editing helpers); `src/ipc/protocol.rs`,
  `src/ipc/server.rs` (`ImportPackage`), `src/api/gui.rs` (`gui_import_package`); `cli/hikasa-cli/src/cmd/gui.rs`;
  `gui/hikasa-gui/src/`: new `options/` (`mod.rs` dialog model, `pages.rs` page table, `commands_page.rs`,
  `theme_page.rs`, `lists.rs` for presets, Open With and archive types), new `ui/options/`, new `wizards.rs` +
  `ui/wizards.rs`, `ipc.rs`, `main.rs` (`.hikasa-pkg`), `bench.rs` (`options`), tests; help new
  `dialog-options.md`, `wizard-export.md`, `wizard-import.md`, `wizard-restore.md`; goldens (IPC frames).

#### 1.4.1 Rules of the GUI wave (D-105)

1. `AppCore` and every model below (`WindowModel`, `ViewSession`, `HomeModel`, dialog models, `Notices`,
   `StatusBar`) contain no egui type; `ui/` only draws them and turns input into `Invocation`s and model calls.
2. Every disk access is a `Service` call made from a job (`Service::spawn`; the result is polled per frame as in
   M1) or from the watch thread; a dialog that waits shows "Loading…"/"Saving…" and stays responsive.
3. Pure core functions are called directly (listed in D-105). No settings, session, filter or comparison logic is
   written in `gui/` (architecture check 7 unchanged; reviewers check the rest).
4. Every new user-visible dialog and view gets a help topic (D-67); F1 in a dialog shows its topic.
5. A command the GUI starts to execute becomes `implemented` in the catalogue in the same part, and
   `docs/reference/commands.md` is regenerated (T14).

#### 1.4.2 Windows, tabs and layout memory (D-106, D-107, D-108)

```rust
// gui/hikasa-gui/src/window.rs (toolkit-free)
pub struct WindowId(pub u32);                     // allocated once, never reused in a process
pub struct WindowModel {
    pub id: WindowId, pub tabs: Vec<Tab>, pub active: usize,
    pub geometry: WindowState,                    // hikasa_session::settings::WindowState (x/y −1 = unknown)
    pub dialogs: DialogStack, pub notices: Notices, pub focused_seq: u64,
}
pub enum TabContent { Live(ViewModel), Pending(PendingTab) }   // Pending: W4b (D-126)
pub struct Invocation { pub id: CommandId, pub arg: Option<CommandArg> }
pub enum CommandArg { Window(WindowId), Tab(usize), SessionType(SessionType), Session(SessionRef),
                      AutoSaved(AutoId), Workspace(SessionName), Page(&'static str), Paths(Vec<PathBuf>) }
impl AppCore {
    pub fn windows(&self) -> &[WindowModel];
    pub fn execute(&mut self, window: WindowId, inv: Invocation) -> Vec<Effect>;
    pub fn focus(&mut self, window: WindowId);                         // focused_seq; triggers a watch poll
    pub fn move_tab(&mut self, from: WindowId, tab: usize, to: Option<WindowId>, index: Option<usize>) -> Vec<Effect>;
    pub fn close_window(&mut self, window: WindowId) -> Vec<Effect>;   // warnings, close actions, root swap
}
pub enum Effect { /* M1: */ TextDirty(ViewId), FolderScroll(ViewId, u32), Exit,
                  OpenViewport(WindowId), CloseViewport(WindowId), Geometry(WindowId, WindowState),
                  ApplyTheme, ApplyCommands }
```

Rules (each a headless test, A4.1):

| Operation | Rule |
|---|---|
| `WindowNew` | a new window with one Home tab; size = `state.toml [window]`; position = the source window's + (32, 32) when known, else unknown |
| `TabNew`, opens | new tab after the active one when `open_new_tab_next_to_active`, else last; an untouched lone Home tab is replaced (M1) |
| target window | IPC `OpenView`/`LoadSession`, the command line, Home: the window with the highest `focused_seq`; `open_sessions_in = "window"` (Home, IPC, command line) and `open_child_sessions_in = "window"` (FC-062 Open) open a new window |
| `TabDuplicate` | a new view of the same session (W4a: the same request; W4b: `session_for_view` → `session_view_of`, origin `Unnamed`) right of the source; Home copies its fields |
| `TabMoveToNewWindow` | available with ≥ 2 tabs in the window; the tab (view, jobs, waiter, layout) moves unchanged |
| `TabMoveToWindow` | submenu of the other windows (caption: the window's active tab caption); appends there and activates it |
| tab drag | reorder inside the bar; release over another window's tab bar → move there; release outside every window → new window there; both only when global positions are known (`ViewportInfo::outer_rect` is `Some` for all windows), else nothing |
| `TabCloseOthers` | closes every other tab of the window; `warn_when_closing_multiple_tabs` and ≥ 2 tabs to close → Prompt "Close N tabs?" first |
| `CloseTab` on the last tab | `closing_last_tab_closes_window` → the window closes, else a Home tab replaces it (M1) |
| a window loses its last tab by a move | the window closes (no Home tab) |
| close a window | `warn_when_closing_multiple_tabs` with > 1 tab → Prompt; then each tab's close action (W4b); the last window → `Exit`; `windows[0]` with others open → root swap (`windows[1]` becomes `windows[0]`, `Geometry(root)` and `CloseViewport(old id)`) |
| `Exit` | `warn_when_closing_multiple_windows` with > 1 window, else `warn_when_closing_multiple_tabs` with > 1 tab → Prompt; then (W4b) the exit workspace save, then every close action (a Cancel stops the exit), then M1 shutdown |
| tab bar | hidden when `hide_tab_bar_if_single_tab` and one tab |
| IPC `Shutdown`, `CloseView` | no prompts (D-120); D-23 outcomes as in M1 |

Viewports: `windows[0]` is drawn in eframe's root viewport; every other window by
`ctx.show_viewport_immediate(ViewportId::from_hash_of(window.id), builder, …)` from the same frame, with title
`<active tab caption> — Hikasa`. A viewport's close request becomes `close_window` (the request is cancelled when a
prompt is shown). Geometry is read each frame from `ViewportInfo` (`inner_rect` size; `outer_rect` position when
`Some`; `maximized`) into `WindowModel::geometry`.

Layout memory (D-108): when a window closes and at exit, `state_update` writes `[window]` from that window's
geometry and `[layout.<view type>]` from the views it held (the last closed view of a type wins). W4 keys:
`toolbar` (bool, true; `ShowToolbar`), `log_panel` (bool, false; `ShowLog`), `log_height` (float points, 160),
`font_size` (float points; the text view's font commands; default the theme's editor size), `thumbnail` (bool,
true; `ShowThumbnail`). `State` gains `layout: BTreeMap<String, toml::Table>` (view type name → table, kept as
is; `settings get state.layout.text-compare` reads it).

#### 1.4.3 Commands: catalogue, overrides, menus, toolbars, shortcuts (D-109, D-110, D-111)

```rust
// hikasa_session::commands (W4a; pure)
pub struct CommandEntry { /* W0 fields */ pub toolbar: bool }      // serde default false; parse and problems unchanged
pub struct EffectiveCommand<'a> {
    pub section: &'a str, pub entry: &'a CommandEntry,
    pub shortcuts: Vec<Shortcut>,            // effective (override or the parsed default; empty when none)
    pub default_shortcuts: Vec<Shortcut>, pub in_menu: bool, pub on_toolbar: bool, pub customized: bool,
}
pub struct ViewCommands<'a> { pub view: &'a str, pub custom: ViewCustomization, pub entries: Vec<EffectiveCommand<'a>> }
pub fn effective<'a>(catalogue: &'a Catalogue, view: &'a str, overrides: &CommandOverrides) -> ViewCommands<'a>;
impl ViewCommands<'_> {
    pub fn get(&self, id: &str) -> Option<&EffectiveCommand<'_>>;           // first in view_sections order
    pub fn with_shortcut(&self, s: &Shortcut) -> Vec<&EffectiveCommand<'_>>; // context-less match, in order
    pub fn conflicts(&self) -> Vec<ShortcutConflict>;                        // same as CommandOverrides::conflicts
}
```

`in_menu` default: `menu` and `caption` non-empty; `on_toolbar` default: `toolbar`. An override's `menu`/`toolbar`
replaces the default; `show_all_in_menu`/`show_all_on_toolbar` force them on for display only. Entries with
status `planned` are in the table (for `settings`), the GUI filters them out.

**Catalogue changes (W4a).** `toolbar = true` on the commands the M1 toolbars show today (unchanged default
toolbars). New `global` entries, `status = "implemented"`, no menu, no shortcut: `TabDuplicate` ("Duplicate
Tab", "Opens current session again in new tab."), `TabMoveToNewWindow` ("Move Tab to New Window", "Moves current
tab into new window."), `TabMoveToWindow` ("Move Tab to Window", "Moves current tab into picked window."),
`TabCloseOthers` ("Close Other Tabs", "Closes all other tabs of this window."). New `home` entries (W4b; no menu;
the sessions tree's context menu, SES-031): `TreeOpen` (Enter), `TreeOpenAsNew`, `TreeOpenAll`, `TreeNewSession`,
`TreeNewFolder`, `TreeClear`, `TreeDelete` (Del), `TreeSaveAs`, `TreeRename` (F2), `TreeEdit`, `TreeExpandAll`,
`TreeCollapseAll`, `TreeFavorite` — captions and descriptions written with the entries (English, original wording).
Statuses become `implemented` per part: W4a `WindowNew`, `ShowToolbar`, `ShowLog`; W4b `SessionNewList`,
`SessionOpen`, `WorkspaceLoadList`, `WorkspaceManager`, `WorkspaceSaveAs`, `SessionHome`, `SessionsCombo`,
`SessionSave`, `SessionSaveAs`, `SessionEdit`, `SessionLocked`, `ShowManage`, `MaximizeManage`, `Find` (home);
W4c `ProgramOptions`, `Export`, `Import`, `Restore`. They stay `planned`: `FileFormatManager`, `EditTextFile`,
`ViewPatch` (W11), `ProfileManager`, `SaveSnapshot` (P1), `Rules`, `Swap`, `SessionClear`, `Info`, `Legend`
(W9/W11), `ShowWeb` (D-133), the Edit commands (their views).

**GUI presentation table** (`commands.rs`). `CommandId` stays a Rust enum with `name()` and `from_name()`; one row
per (scope, id): `icon: Option<IconId>`, `toolbar_label: Option<&str>` (short label for large buttons),
`group` (radio/check group), `enabled_when: Predicate`, `areas: &[Area]` (`Area::{Tab, Toolbar, SessionTree}`;
W9+ add pane areas). Captions, hints (= the catalogue `description`), menus and shortcuts are read from
`ViewCommands`. The window rebuilds the effective tables (one per view type) at start and on
`Effect::ApplyCommands` (a `commands.toml` change or Apply), never per frame.

**Menus, toolbar, context menus** follow D-110. The Session menu's dynamic entries: `SessionNewList` (one item per
session type the GUI can open), `WorkspaceLoadList` (saved workspaces, then Manage Workspaces…). The View menu
holds `ShowToolbar` and the view's Display Filter submenu. The toolbar shows the visible `on_toolbar` entries
with an icon (and the label under it with `large_buttons`), plus the display filter control of
`display_filter_mode`, plus `SessionsCombo` for session views (W4b). **Toolbar right-click menu** (UI-004):
radio Dropdown / Favorites / Toggles, check "Large buttons with text labels", "Toolbar Options…"
(`ProgramOptions` with `CommandArg::Page("commands")` and the view preselected; W4c, before that hidden), "Hide
Toolbar" (`ShowToolbar`). The first two write `commands.toml` (`settings_save`, a job). **Tab context menu:**
`TabNew`, `TabDuplicate`, `TabMoveToNewWindow`, `TabMoveToWindow ▸`, `TabCloseOthers`, `CloseTab`.

**Hints** (UI-007): tooltip = caption, then the first effective shortcut in parentheses when
`show_shortcut_in_hints`, then the description on a second line; none when `show_hints` is off; shown after
`hint_show_ms` (egui `tooltip_delay`), hidden after `hint_hide_ms` of hover (0 = never; the UI layer tracks the
hover start).

**Keys** (D-111): `ui/keys.rs` maps egui keys and modifiers to `Shortcut` (letters upper case, digits, `F1`…`F24`,
the named keys of W1, keypad keys where egui distinguishes them); a key egui cannot report is simply not
bindable (the Shortcuts page refuses to capture it). The focus areas of W4: the window root, a view's main area
(folder tree, text panes), a path field, the sessions tree, the Home search box, a dialog.

#### 1.4.4 Dialog framework and prompts (D-112)

```rust
// gui/hikasa-gui/src/dialog.rs
pub trait DialogModel {
    fn title(&self) -> String;  fn help_topic(&self) -> &'static str;
    fn buttons(&self) -> Vec<DialogButton>;           // label, role (Ok|Apply|Cancel|Yes|No|Back|Next|Finish|Other), enabled, default
    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep;   // Stay | Close | CloseWith(Vec<Invocation>)
    fn on_key(&mut self, key: &Shortcut) -> Option<ButtonRole> { /* Esc → Cancel/Close, Enter → default */ }
    fn poll(&mut self, svc: &Service) -> bool { false }   // jobs of the dialog
}
pub struct DialogStack { /* Vec<Box<dyn DialogModel>> */ }
pub struct Prompt { pub message: String, pub buttons: Vec<(ButtonRole, String)>, pub default: ButtonRole,
                    pub do_not_ask: Option<&'static str> /* options key set to false when ticked */ }
```

The top dialog of a window receives the keys and pointer of that window; the window's commands are disabled while
the stack is not empty (except `Exit` from the OS close button, which first closes the dialogs with Cancel).
`About` shows a dialog (version, the line "every function is also available from the hikasa command line"). The
prompts W4 uses: the multiple-close warnings (with "Do not ask again" → `[tabs] warn_when_closing_multiple_*`),
D-84 "Save changes to session?" (Yes/No/Cancel) and "Save changes to locked session?" (Yes/No), settings
conflicts (D-128), overwrite of an existing session, workspace or package file, deletes in the Home tree.

#### 1.4.5 Theme application and icons (D-113, D-114)

- `ui/theme.rs`: `Palette::from_scheme(&Scheme) -> Palette` (replaces the two hard-coded palettes); one table
  `(element name, Palette field)` covers every field; the folder tree paints `[folder]` (FC-031 classes →
  `same`, `newer`, `older`, `orphan`, `different`, `unknown`, `filtered_out`, `stripes`, `selection`, …), the
  text panes paint `[file]` (backgrounds of important/unimportant lines and text, orphan, gutter, line numbers,
  current line, selection, find highlight, syntax colours through `Scheme::syntax_colour`), the log panel
  `log_panel`. `Visuals::light()`/`dark()` by `Scheme::base_of`, selection from `[folder] selection` unless
  `use_system_selection`.
- The theme is (re)applied at start, when the OS theme changes under `follow_os`, on `Effect::ApplyTheme`
  (a theme file changed on disk, Apply, a preview step of D-130) — never per frame.
- Fonts (D-113): editor size → text panes; folder size and `use_system` → tree (system = egui's proportional at
  its default size); listing and hex sizes are stored for W11/W13. Font size commands of the text view scale from
  the editor size and are remembered in `[layout.text-compare] font_size`. Families: D-160; the text panes and the tree
  draw with their slot's family, whose chain ends in the built-in fonts.
- HiDPI (UI-010): egui follows each monitor's scale factor (winit); custom painters work in points; icons are
  vector.

**Icon inventory (W4; `ui/icons.rs`, `IconId`).** Session types: `FolderCompare`, `FolderSync`, `TextCompare`,
`TextEdit`, `TableCompare`, `HexCompare`, `PictureCompare`. Window and session: `Home`, `NewTab`, `NewWindow`,
`Close`, `Open`, `Save`, `SaveAs`, `Settings`, `Lock`, `Unlock`, `Favorite`, `Folder`, `Session`, `AutoSaved`,
`Defaults`, `Workspace`. Actions: `Stop`, `Reload`, `Expand`, `Collapse`, `ExpandAll`, `CollapseAll`,
`OpenPair`, `NextDiff`, `PrevDiff`, `NextSection`, `PrevSection`, `Swap`, `Rules`, `Search`. Display filters:
`FilterAll`, `FilterDiff`, `FilterSame`, `FilterOrphans`, `FilterLeftOrphan`, `FilterRightOrphan`,
`FilterLeftNewer`, `FilterRightNewer`, `FilterNone`, `Context`, `Unimportant`. Panels and state: `Log`,
`Thumbnail`, `FontLarger`, `FontSmaller`, `FontReset`, `Info`, `Warning`, `Error`, `Notice`, `Drop`, `Options`,
`Export`, `Import`, `Restore`, `Help`. Each is `&'static [Op]` (`Op::{Line, Polyline, Polygon, FilledPolygon,
Circle, FilledCircle, Rect, FilledRect}` with coordinates in `0.0..=16.0`, stroke 1.5 units scaled with the icon);
`paint(painter, rect, id, colour)`; `IconId::ALL`. Later waves add icons to the same table (W9 centre icons, W13
picture tools).

#### 1.4.6 Status bar, jobs, message panel, log panel (D-115, D-116)

```rust
// gui/hikasa-gui/src/status.rs
pub struct StatusItems { pub left: String, pub right: Vec<String> }   // ViewModel::status() per view
pub struct JobStatus { pub view: ViewId, pub title: String, pub phase: Option<Phase>, pub fraction: Option<f32> }
pub struct Message { pub kind: MessageKind, pub text: String, pub do_not_show: Option<&'static str> }
// ViewModel gains: status(), jobs() -> Vec<JobStatus>, cancel_jobs(), message(), dismiss_message()

// hikasa_session::log (pure)
pub enum LogKind { Summary, Detail, Warning, Error, Debug }
pub struct LogEntry { pub time: jiff::Zoned, pub kind: LogKind, pub source: String, pub text: String }
pub struct LogFilter { pub summaries: bool, pub details: bool, pub warnings: bool, pub debug: bool, pub capacity: usize }
impl LogFilter { pub fn from_options(o: &Options) -> LogFilter; }        // OPT-018
pub struct LogBook { /* VecDeque<LogEntry>, filter, generation, unsaved lines */ }
impl LogBook {
    pub fn new(filter: LogFilter, save: bool) -> LogBook;  pub fn set_filter(&mut self, f: LogFilter, save: bool);
    pub fn push(&mut self, e: LogEntry) -> bool;            // false: filtered out
    pub fn push_warnings(&mut self, time: &jiff::Zoned, source: &str, w: &[Warning]);
    pub fn entries(&self) -> impl Iterator<Item = &LogEntry>; pub fn generation(&self) -> u64; pub fn clear(&mut self);
    pub fn take_unsaved(&mut self) -> Vec<String>;          // formatted lines since the last call (save on)
}
pub fn format_line(e: &LogEntry) -> String;                 // "2026-10-30 14:03:07  warning  <source>: <text>"
```

The job indicator lists `jobs()` of every view of the window; Cancel on a row cancels that view's jobs (the
model's `cancel` as M1 Stop). Job summaries for the log come from each view's finished job (source = tab caption;
text = the outcome and counts the view already shows, and the duration). Warning codes keep their stable code in
the text (`[unknown_config_key] …`). The log panel is a bottom panel of the folder view (resizable,
`log_height`), newest last, follows the end unless scrolled up, *Clear* button; colours `log_panel` and the
warning/error colours of the scheme.

#### 1.4.7 Notifications and polling (D-117, D-118, D-119)

```rust
// hikasa_session::watch (pure + VFS stat)
pub struct InputStamp { pub len: u64, pub modified_ns: Option<i128> }
pub enum InputState { Unwatched, Missing, Present(InputStamp) }
pub struct InputWatch { /* spec → InputState */ }
impl InputWatch { pub fn track(&mut self, spec: String, now: InputState); pub fn untrack(&mut self, spec: &str);
                  pub fn poll(&mut self, svc: &Service) -> Vec<(String, ChangeKind)>; }
// gui/hikasa-gui/src/notify.rs
pub enum NoticeKind { Info, Warning, Error }
pub enum NoticeAction { ReloadView(ViewId), ReloadSettings, KeepMine, ShowTab(ViewId), OpenOptions, Dismiss }
pub struct Notice { pub key: String, pub kind: NoticeKind, pub text: String, pub actions: Vec<NoticeAction> }
pub struct Notices { /* per window; push(now), visible(now), act(index, action) */ }
```

The watch thread owns a `ConfigWatch` (`Service::settings_store().watch()`) and an `InputWatch`; the UI sends it
`Poll` (window focus), `Track`/`Untrack` (file views opening and closing), `Acknowledge(paths)` (after each own
write, before the write's job returns its result to the UI), `MaterializeFormats` (the first-run format files of
D-21, written and acknowledged by the watch thread itself), `Stop` (exit). It sleeps with
`recv_timeout(2 s)`, so a poll runs at most every 2 s plus on focus. Tests drive `Watcher::poll_once` directly.

| Change | Reaction |
|---|---|
| `options.toml` | `reload_settings` (job) → the window applies tabs, interface, hints, metrics, log filter; notice info "Settings reloaded: options.toml changed on disk." (a problem: warning notice naming the keys reset to defaults) |
| `commands.toml` | effective tables rebuilt (`ApplyCommands`); info notice |
| `theme.toml`, `themes/*.toml` | theme re-applied (`ApplyTheme`); info notice |
| `formats/*.toml`, `order.toml` | `reload_settings` (registry); info notice "…; open comparisons use the new formats when reloaded" |
| `filters.toml`, `openwith.toml`, `archives.toml` | snapshot refreshed; info notice |
| `state.toml` | nothing (merged by `state_update`) |
| the session store | Home and the menus read the sessions again (a job, no notice; D-124) |
| a file removed | as changed; the text says "removed: defaults apply" |
| a file the Options dialog has unapplied edits for | warning notice with *Reload (discard my edits)* / *Keep my edits* (the next Apply then meets `Conflict`, D-128) |
| an input of a file view changed | D-118: reload + info notice, or warning notice with *Reload* |
| an input removed | warning notice "<path> was removed or renamed." |
| startup warnings, `LoadWorkspace` warnings | warning notices |
| a job of a background tab failed | error notice with *Show* |

Notices are timed with the frame time (`ctx.input(|i| i.time)`), so tests pass explicit times. Later waves add
sources: W5 archive cache limits, W6 operation results, W8 script end, W10/W11 edited files changed on disk.

#### 1.4.8 Drag and drop from the OS (UI-008)

`dnd.rs`: `DropTarget::{PathField(FieldId), HomeType(SessionType), Window}` and
`AppCore::drop_paths(window, target, paths) -> Vec<Effect>`. A path field takes the first path (two paths on the
left field of a pair: left and right). The window (any view other than Home's targets) with two paths opens a
comparison like the command line (`resolve_open` in a job), with one path does nothing but a message panel hint,
with more than two a message "Drop one or two files or folders.". Home type buttons (W4b): one path → a new
session of that type with the left side filled, shown in the management panel; two paths → that type's view
(`open_view_request`; a mismatch such as files on Folder Compare → the panel's error). The UI layer reads
`raw.dropped_files` (paths only; bytes-only drops are ignored) and highlights the target under
`raw.hovered_files`. Delivery depends on the platform backend (winit); the model is tested headless, the delivery
manually.

#### 1.4.9 Sessions of views: save, lock, close prompts, auto-save (W4b; D-120, D-121)

```rust
// gui/hikasa-gui/src/view_session.rs
pub enum ViewOrigin { Unnamed, AutoSaved(AutoId), Named(SessionRef) }
pub struct ViewSession { pub origin: ViewOrigin, pub loaded: Option<Session>, pub locked: bool,
                         pub explicit_save: bool, pub last_autosaved: Option<Session>,
                         pub parent: Option<ParentLink>, pub next_autosave_at: f64 }
```

| Command / event | Behaviour |
|---|---|
| open from Home *New*, a drop, IPC `OpenView`, the command line | origin `Unnamed` |
| open a named session (tree, `SessionsCombo`, IPC `LoadSession`, workspace) | origin `Named`, `loaded` = the effective session (D-82), `locked` from the file |
| open an auto-saved session | origin `AutoSaved(id)`, slot = id (later auto-saves overwrite it, D-83) |
| FC-062 Open of a pair | origin `Unnamed`, `parent` = the folder view's reference (named or auto-saved) and relative paths (TC-052) |
| `SessionSave` | `Named` and not locked → `session_save`; `Named` and locked → Prompt "Save changes to locked session?" → Yes: `session_save(force)`; otherwise → Save Session As |
| `SessionSaveAs` | dialog: name, folder (existing folders, new folder), existing names offered (overwrite → Prompt); `session_save` → origin `Named`, the auto-saved slot of the view is deleted (`session_delete_autosaved`) |
| `SessionLocked` | toggles with `session_lock` (named only; check state = `locked`) |
| `SessionHome` | the view's close action, then a Home tab in its place |
| `SessionsCombo` | the named sessions of the view's type; choosing one runs the close action and opens it in this tab |
| close (tab, window, exit, Load Workspace) | `close_action(origin, locked, dirty, explicit_save, auto_save_on)` → `AutoSave` (`session_autosave`), `SaveViewState` (`session_save` of `view`/`display` only), `AskSaveChanges` (Prompt Yes/No/Cancel), `AskSaveLocked` (Prompt Yes/No), `Nothing`; a Cancel stops the whole close (the tab, the window, the exit or the load) |
| IPC `CloseView`, `Shutdown` | as close, every Ask answered No (D-120) |
| timer | every 300 s per `Unnamed`/`AutoSaved` view whose current session differs from `last_autosaved` (D-121); jobs run in the background, at most one per view |

`dirty` and the current session come from `Service::session_for_view` (specs as stored, D-80; the rules the view
was built from; `view` = the layout keys of §1.4.2 the view owns; `display` = its display filter state).

#### 1.4.10 Session Settings dialog shell (W4b; D-122, D-123)

`SessionEdit` opens the dialog for the active view; `TreeEdit` for a named session or a defaults entry (then OK
writes the file through `session_save`/`session_save_defaults` and no view is involved). Registry:
`fn tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>>` with `SessionTab { title, help_topic, changes_rules(),
draw-independent model methods }`. **Specs tab:** left and right (MRU drop-downs from `state.mru.left/right`,
pushed on OK), description, for file types format (*Detect* + `format_list`) and the two encodings (empty =
detect; checked with `TextEncoding::for_label`); for a defaults entry the specs are disabled (D-82). **Footer:**
the scope radio buttons with availability = `Pair`/`Type` only when `parent` is a named or auto-saved folder
session; all but *this view only* disabled while no rule tab changed anything. **OK:** specs changed → the view
reopens with the new specs (`open_view_request`, origin kept, dirty from then on); rules changed →
`session_apply_scope(scope, …)` and the view re-runs with the effective rules. **Cancel** discards. F1 →
`dialog-session-settings`.

#### 1.4.11 Home view (W4b; D-124)

```rust
// gui/hikasa-gui/src/home.rs
pub enum TreeNode { NewType(SessionType), Folder(String), Named { reference: SessionRef, kind: SessionType,
                    description: String, locked: bool, favorite: bool }, AutoGroup(AutoGroup),
                    AutoSaved { id: AutoId, kind: SessionType, label: String } }
pub struct HomeModel { pub tree: Vec<TreeRow>, pub search: String, pub selected: Option<usize>,
                       pub expanded: BTreeSet<String>, pub panel: ManagementPanel, pub panel_mode: PanelMode /* Shown|Hidden|Maximized */ }
impl HomeModel { pub fn load(&mut self, list: &SessionListDto); pub fn set_search(&mut self, text: &str);
                 pub fn visible(&self) -> Vec<&TreeRow>; pub fn drop_paths(…); }
```

| Tree command (SES-031) | Applies to | Service |
|---|---|---|
| `TreeOpen` (Enter, double-click) | named, auto-saved, *New* type | `session_view` / `session_load` + `session_view_of` / an empty session in the panel |
| `TreeOpenAsNew` | named, auto-saved | `session_load` → origin `Unnamed` |
| `TreeOpenAll` | folder | `session_view` for each named session directly in it |
| `TreeNewSession`, `TreeNewFolder` | folder or root | name prompt → `session_new` / `session_mkdir` |
| `TreeClear` | named (not locked) | `session_defaults` → `Session::clear` → `session_save` |
| `TreeDelete` (Del) | named, folder, auto-saved | Prompt → `session_delete` / `session_rmdir(recursive)` (the prompt names the count and the locked sessions, confirming is the force, D-91) / `session_delete_autosaved` |
| `TreeSaveAs` | named, auto-saved | Save Session As → `session_load` + `session_save` under the new reference |
| `TreeRename` (F2) | named, folder | inline edit → `session_rename` / `session_rename_folder`; locked (a folder: any session below it) → Prompt, then `force` |
| `TreeEdit` | named, defaults | Session Settings dialog (§1.4.10) |
| `TreeExpandAll`, `TreeCollapseAll` | tree | — |
| `TreeFavorite` | named (not locked) | `session_edit` (`favorite = true/false`) |
| drag a named session onto a folder | named | `session_rename` (move; locked → Prompt, then `force`) |

The management panel shows the selection: a *New* type → an empty session of that type (Specs editor, Compare);
a named session → description, Specs editor, Locked, buttons Save (`session_save`; on a locked session a plain refusal, the panel does not offer the force), Save As…, Factory Defaults
(the panel's copy becomes `Session::clear`; written on Save), Cancel (revert), Open; an auto-saved one → read-only
Specs, Open, Open as New, Delete. `HomeCompare` = Open of the selection with the panel's specs. The M1 refusals
of `-` stay; `clipboard://` is accepted (D-101: the GUI reads its own clipboard when the view opens).

#### 1.4.12 Workspaces, startup and exit, command line (W4b; D-125, D-126, D-127)

- **Save Workspace As** (`WorkspaceSaveAs`): name combo (existing names offered; overwrite → Prompt); capture of
  D-125 → `workspace_save`; `state.mru.workspaces` pushed (OPT-019).
- **Load Workspace** (`WorkspaceLoadList` item, a workspace shortcut, IPC `LoadWorkspace`, startup):
  `workspace_load` (job) → close actions of every open view (D-120; Cancel aborts) → windows beyond the
  workspace's count close, `windows[i]` take the workspace's windows (geometry applied as allowed), tabs become
  `Pending` except each window's active tab, which starts. Warnings → notices and log.
- **Manage Workspaces** (`WorkspaceManager`): list (name, windows, tabs, shortcut); Load, Rename
  (`workspace_rename`), Delete (Prompt, `workspace_delete`), Shortcut (typed as text and checked with `Shortcut::parse`; `workspace_load` + modified
  `Workspace` + `workspace_save`); conflicts with the effective `global` shortcuts and other workspaces are shown,
  never blocking.
- **Startup and exit** (OPT-001): as D-125; the startup load runs after the first frame (like D-21's deferred
  work) so the window appears first.
- **Command line** (D-127): `main.rs` classifies a single non-path argument with `classify_single_arg` before
  the M1 path handling; forwarded to a running GUI as `LoadSession`/`LoadWorkspace` (W4b) or `ImportPackage`
  (W4c); `--silent` with a package keeps the headless import (W2).

#### 1.4.13 Options dialog (W4c; D-128, D-129, D-130)

```rust
// hikasa_session::settings (W4a; used by W4a's toolbar menu and W4c)
#[derive(Clone)] pub struct SettingsSnapshot {
    pub options: SettingsFile<Options>, pub commands: SettingsFile<CommandOverrides>,
    pub theme: SettingsFile<ThemeSelection>, pub schemes: BTreeMap<String, SettingsFile<SchemeFile>>,
    pub filters: SettingsFile<FilterPresets>, pub openwith: SettingsFile<OpenWith>, pub archives: SettingsFile<ArchiveTypes>,
    pub unreadable: BTreeMap<FileId, String>,   // W4 leftovers, §1.4.20
}
#[derive(Default)] pub struct SettingsChanges {
    pub options: Option<Options>, pub commands: Option<CommandOverrides>, pub theme: Option<ThemeSelection>,
    pub schemes: BTreeMap<String, Option<SchemeFile>>,       // None: delete themes/<name>.toml
    pub filters: Option<FilterPresets>, pub openwith: Option<OpenWith>, pub archives: Option<ArchiveTypes>,
}
pub enum FileOutcome { Written, Unchanged, Deleted, Conflict, Unreadable(String), Invalid(Vec<Problem>), Failed(String) }
pub struct SettingsSaveDto { pub files: Vec<(FileId, PathBuf, FileOutcome)>, pub snapshot: SettingsSnapshot, pub reread_error: Option<String> }
// CommandOverrides helpers (W4c; pure)
impl CommandOverrides {
    pub fn set_shortcuts(&mut self, view: &str, id: &str, s: Option<Vec<Shortcut>>);   // None = Default
    pub fn set_in_menu(&mut self, view: &str, id: &str, v: Option<bool>);
    pub fn set_on_toolbar(&mut self, view: &str, id: &str, v: Option<bool>);
    pub fn view_mut(&mut self, view: &str) -> &mut ViewCustomization;                // created on demand
    pub fn reset_view(&mut self, view: &str);
}
```

`settings_save` processes the files in `FileId::every` order, each independently: unchanged value → `Unchanged`;
`validate` problems → `Invalid` (not written); else `SettingsFile::save(IfUnchanged | Force)` → `Written` /
`Conflict`; a deleted scheme file → `Deleted`; and returns a fresh snapshot (when that read fails, the given snapshot
with the saved files in it, and `reread_error`: the outcomes hold all the same). It is the same `SettingsFile` path as
`settings set` (A4.17 compares the files).

**Pages** (`options/pages.rs`, data; label wording original, after `docs/15`):

| Page id | Title | Content |
|---|---|---|
| `startup` | Startup | OPT-001 (workspace combos from `workspace_list`; the Quick Compare keys note "used from W13") |
| `tabs` | Tabs | OPT-004 P0 keys |
| `appearance` | Theme, Colors, Fonts | `theme_mode`, light and dark scheme; sub-pages `appearance.folder`, `appearance.file`, `appearance.picture`, `appearance.fonts` (D-130) |
| `text_editing` | Text Editing | OPT-006 |
| `next_difference` | Next Difference | OPT-007 |
| `backups` | Backups | OPT-008 |
| `file_operations` | File Operations | OPT-009 |
| `archive_types` | Archive Types | OPT-010 (`archives.toml`; mask per type, restart note) |
| `commands` | Toolbars, Shortcuts, Menus | OPT-011 (D-129) |
| `open_with` | Open With | OPT-012 list (New / Edit / Clone / Delete; item dialog checked with `OpenWith::validate` and `split_words`) |
| `tweaks.editor_display` … `tweaks.scripts` | Tweaks ▸ Editor Display, File Operations, Interface, Log Panel, Metrics, Name Filter Presets, Scripts | OPT-014, OPT-015, OPT-017, OPT-018, OPT-019, OPT-020 (`filters.toml` list), OPT-021 |

Widgets by `ValueKind`: Bool → check box; Int/Float → drag value clamped to the range; Enum → combo with a caption
per value (a test checks every value has one); String/Path → text field (drop accepted; Browse in W9); Mask →
text field checked with `MaskList`; Colour → colour button; Shortcut → capture field; lists → the page's editors.
The dialog opens on the page of `CommandArg::Page` or the last page used (`state.dialogs.options.page`).

#### 1.4.14 Export, Import and Restore wizards (W4c; D-131, D-127)

| Wizard | Steps | Service |
|---|---|---|
| Export (`Export`) | 1 categories (Program Options, Theme, Toolbars/Shortcuts/Menus, Program State, Sessions all/some, File Formats all/some, Workspaces all/some, System Info) with item pickers; 2 target file (overwrite → Prompt); result | `session_list`, `format_list`, `workspace_list`; `settings_export` |
| Import (`Import`, D-127) | 1 package path (prefilled from the command line or IPC); 2 categories present in the package and their items (action Add, Replace in red, Kept for locked sessions; "Include locked sessions" = force), delete-existing choices for sessions, formats, workspaces; 3 summary ("12 sessions will be imported, replacing 3 existing." per category); result per item | `settings_import(dry_run = true)` for steps 2 and 3, `settings_import(dry_run = false)` at Finish |
| Restore (`Restore`) | 1 categories, "Delete all named sessions", "Delete all workspaces", "Include locked sessions"; 2 the files to delete; result | `settings_restore_categories(dry_run = true / false)` |

A package that does not validate shows the core message on step 1 and nothing can proceed. After Finish the written
and deleted paths are acknowledged with the watch thread, then the settings reload path of §1.4.7 runs and the Home
tree refreshes.

#### 1.4.15 Service calls of the GUI

*Exists* = available at `1fc12e2`; *new* = added in core by the named part (D-105: typed forms of existing CLI
operations, or GUI-only state). Pure functions the GUI calls directly are not listed.

| GUI need | Call | Status |
|---|---|---|
| comparisons, IPC, M1 | `text_compare`, `folder_prepare`/`folder_run`, `spawn`, `open_view_request`, `resolve_open`, `forward_open`, `endpoint`, `read_clipboard`, `startup_warnings` | exists |
| sessions of views | `session_view` (named, effective), `session_load(&SessionRef) -> Loaded<Session>`, `session_defaults(SessionType) -> Session`, `session_save(&SessionRef, &Session, force) -> SessionWriteDto`, `session_save_defaults(&Session)`, `session_autosave(Option<&AutoId>, &Session) -> Option<AutoId>` (clock, prune; `None` when off; a locked slot is returned unwritten, so the GUI job reads the slot first), `session_delete_autosaved(&AutoId)`, `session_for_view(&ViewRequest, view: Table, display: Table, parent: Option<ParentLink>) -> (Session, Vec<Warning>)`, `session_view_of(&Session, title) -> SessionView`, `session_apply_scope(Scope, &SessionRules, &mut Session, parent: Option<&SessionRef>) -> ScopeWritesDto` | `session_view` exists; the others new (W4b) |
| Home tree and management | `session_list(None, true, true)`, `session_new`, `session_edit` (favorite), `session_delete`, `session_rename`, `session_lock`, `session_mkdir`, `session_rmdir(folder, recursive, force)`, `format_list`; `session_rename_folder(from, to, force)`, `session_folder_contents(folder)`, `session_prune_autosaved()` | exist; the last three new, `force` added to `session_rmdir` (W4b, D-91) |
| workspaces | `workspace_list`, `workspace_save`, `workspace_delete`, `workspace_rename`, `classify_single_arg`; `workspace_load(&SessionName) -> WorkspaceLoadDto { workspace, windows: Vec<RestoredWindow>, warnings }`, `inline_table(&Session, previous: Option<&Table>) -> Table` | first five exist; last two new (W4b; wrap `workspaces::restore` and `inline_table`) |
| settings for the window and the dialogs | `settings_snapshot() -> SettingsSnapshot` (lenient; a newer schema is an error), `settings_snapshot_partial()` (a file that cannot be read is named instead, §1.4.20), `settings_save(&SettingsSnapshot, &SettingsChanges, force) -> SettingsSaveDto`, `reload_settings() -> Vec<Warning>`, `key_infos` (pure), `settings_store().watch()` | `settings_store` exists; the others new (W4a) |
| state and layout | `state_update(&mut dyn FnMut(&mut State)) -> Result<bool, ApiError>` | new (W4a) |
| log and inputs | `log_append(&Path, &[String])` (append, create the parent folder; not atomic), `input_stamp(&str) -> InputState` | new (W4a) |
| packages | `settings_export`, `settings_import`, `settings_restore_categories` | exist |
| IPC client (CLI) | `gui_import_package(&Path, launcher, policy)` | new (W4c) |

`Service::options()` and `registry()` return snapshots (D-119). The IPC handler of the GUI gains
`ImportPackage` (W4c).

#### 1.4.16 View-model tests without a window

Tests construct `AppCore` with a `Service` over a temporary configuration (`ServiceInit` with a fixed clock,
`FixedClipboard`, a private IPC endpoint) and drive it only through `execute(window, Invocation)`, `move_tab`,
`drop_paths`, dialog models (`on_button`, `on_key`), `Watcher::poll_once` and `poll()`; jobs are awaited by
polling (bounded). Rules:

1. **Catalogue parity** (extends A0.2): every `implemented` entry of every section a shown view type uses is
   executed by the GUI (an `Invocation` with a test argument where needed changes the model or opens a dialog),
   and every id the GUI executes is `implemented` with the same default shortcut; every presentation row with
   `toolbar` has an icon.
2. **Menus as text:** a golden per view type (Home, Folder Compare, Text Compare) of the menu bar rendered by
   `menus.rs` as indented text (separators, check/radio state, shortcuts), before and after a fixture
   `commands.toml`.
3. **Shortcuts:** a table test of D-111 (focus areas, own over global, overrides, removed shortcuts, contexts, a
   workspace shortcut, dialogs open).
4. **No egui in models:** the architecture test's check that `app.rs`, `window.rs`, `commands.rs`, `menus.rs`,
   `dialog.rs`, `status.rs`, `notify.rs`, `view_session.rs`, `home.rs`, `workspace.rs`, `session_dialog.rs`,
   `wizards.rs` and `options/*.rs` do not name `egui`/`eframe` (new rule of check 7).
5. **Shapes, not pixels:** where drawing matters (icons, theme colours), a headless `egui::Context::run_ui` produces
   the shapes, and tests check their bounds and colours; no screenshot comparison.

#### 1.4.17 Performance gates and measurement (D-132)

| Gate | Bench mode (part) | Scenario | Threshold |
|---|---|---|---|
| ARCH-GUI-003 (unchanged) | `scroll` (W4a, again at the end of W4c) | the M1 1M-line C pair, three scroll phases | first screen < 1 s; PSS < 381.5 MiB and peak RSS reported; UI frame work p95 < 8 ms, max < 16.7 ms |
| two windows | `two_windows` (W4a) | the same comparison in window 1, then `WindowNew` (Home) as window 2; the three scroll phases in window 1 | PSS < 381.5 MiB; frame work as above |
| Options open | `options` (W4c) | one window with Home; `ProgramOptions`, every page shown once, then 60 idle frames | PSS < 120 MiB |
| G12 workspace | `workspace` (W4b) | a workspace of 10 tabs in one window: active tab the 1M-line session, then 5 text sessions of 10,000-line pairs and 4 folder sessions of 1,000-file trees; `hikasa-gui <workspace>` | first screen of the active tab < 1 s after the process start; no job started for the 9 pending tabs; memory per pending tab < 20 MiB: the same workspace with a small active pair (1,000 lines) against one holding only that active tab, same files, both in `workspace` mode and sampled after the coloured screen with no job left and 60 idle frames: (PSS_10 − PSS_1) / 9, the anonymous figure beside it |

Measurement: `env -u WAYLAND_DISPLAY xvfb-run -a cargo test --release -p hikasa-gui --test perf_gate --
--ignored --nocapture --test-threads 1` (new tests `two_windows`, `options_memory`, `workspace_restore`; the
existing `text_scroll` and `folder_scan` stay). Each summary carries `renderer`; a test fails when `DISPLAY` is
unset or the adapter is not a software renderer (llvmpipe), so no run uses the real display. Memory as in docs/02
(PSS from `smaps_rollup`, MiB; peak RSS from `VmHWM`). The figures, machine and load are recorded in
`docs/02` (W4 section); a gate that fails stops the part and is reported (`docs/17` §7.1); a failure caused only
by the software renderer is reported with that evidence, never measured again on the real display by the agent.

#### 1.4.18 Hooks for later waves

- **Dialogs** (D-112): W6 confirmation dialogs and the delete dialog, W8's script status window, W9's VFS browse
  dialog, W11's per-file unsaved-edits prompt (SES-092), W13's report and Quick Compare dialogs are `DialogModel`s.
- **Notices and the log** (D-116, D-117): W5, W6, W8, W10/W11 add sources; W6 file operations append summaries and
  per-item details.
- **Session Settings tabs** (D-122): W9 folder rules tabs, W11 text tabs (with D-123), W13 table/hex/picture tabs
  register in the same registry; `Rules` becomes `implemented` with the first one.
- **Context-menu areas** (D-110): W9 folder panes, W11 text panes and gutter, W13 grids.
- **Layout keys** (D-108) and icons (D-114) grow with their views.

#### 1.4.19 Documentation updates of W4

`docs/14` §1 (window model, tab moves, menus, shortcut dispatch, hints, UI-008 targets: D-106…D-111), §2 (theme
application, fonts, live preview: D-113, D-130), §3 (status bar, jobs, message panel, log, notifications and
their sources: D-115…D-118), §4 (the dialog framework, D-112); `docs/15` (the Options dialog: pages, search,
Apply/OK/Cancel, conflicts: D-128, D-129); `docs/03` §1 (periodic auto-save, non-interactive closes: D-120,
D-121), §2 (dialog shell and scopes: D-122; the D-123 note), §3 (Home: D-124), §4 (workspaces in the GUI, lazy
restore: D-125, D-126), §5.2 (`[layout.<view>]`, D-108), §6 (the wizards, D-131), §9 (tabs and windows); `docs/04`
FC-100 (the log panel and book, D-116); `docs/01` §3.6 (effective command table, D-109; `reload_settings`, D-119)
and §5.1 (`ImportPackage`, D-127); `docs/12` §1 (D-88 package wizard now available); `docs/02` (the W4
measurements, D-132); `docs/16` M2 progress; `docs/reference/commands.md` regenerated; help topics
`main-window`, `view-home`, `dialog-session-settings`, `dialog-save-session`, `dialog-workspaces`,
`dialog-options`, `wizard-export`, `wizard-import`, `wizard-restore` (and `help/gui.md` for `ImportPackage`),
linked from `index`/`main-window`; `CHANGELOG.md` per part.

#### 1.4.20 W4 notes (implementation)

*W4 note:* where the implementation had to interpret or extend the text above:

**W4a**

- **Merged entry texts.** The `text-compare` entries of the display filters refine the `file-shared` ones and
  have no menu and no caption of their own, so `EffectiveCommand` also carries `menu`, `caption` and
  `description`: the first non-empty value over the view's sections in `view_sections` order (`entry` stays the
  first entry). Without this the text view's filters would vanish from the menus. The default shortcut is merged
  the same way (`ExistsNext`/`ExistsPrev` of the text, table and hex views take `Alt+Down`/`Alt+Up`, `SelectAll`
  takes `Ctrl+A`, `DiffItemNext`/`Prev` of the table and hex views `Ctrl+N`/`Ctrl+P`). `status` and `toolbar`
  are not merged: they are the view's own entry's (`entry`), because a view-specific entry exists precisely to say
  whether that view implements the command (the text view's display filters are `implemented` there while the
  shared entries are `planned`); no catalogue entry differs in `toolbar` across sections.
- **Default toolbars.** `toolbar = true` on `DiffNext`, `DiffPrev`, `Stop`, `Open`, `ExpandAll`, `CollapseAll`,
  `FullRefresh`, `Reload`, `DiffSectionNext`, `DiffSectionPrev`, `InsigIgnored` (the M1 buttons). The presets and
  toggles are not flagged: they are the display filter control, which by `display_filter_mode` shows a drop-down
  of every preset with the toggle buttons (`dropdown`, the M1 toolbar unchanged), the `Favorites` presets as
  buttons beside a drop-down of the others (`favorites`), or the toggle buttons alone (`toggles`). The folder
  mode stays a drop-down. Entries without a catalogue caption (the toggles) take the presentation's short label
  as caption in the Display Filter submenu.
- **Dialog follow-ups.** `DialogStep::CloseWith` carries `Followup`s — `Run(Invocation)` (runs without asking
  again), `CloseWindow`, `Exit`, `DisableOption(key)` — instead of plain invocations: closing a window and quitting
  after a confirmed warning are not catalogue commands, and a confirmed command must not prompt again. "Do not ask
  again" and "Do not show again" write their key with `settings_set` (the CLI's path) on a job; the window follows
  at once in memory.
- **`open_sessions_in = "window"`** opens a new window unless the focused window holds only an untouched Home tab,
  which the view replaces (the first command-line view would otherwise leave an empty Home window).
- **`TabDuplicate`** inserts right of the source whatever `open_new_tab_next_to_active` says (D-107 "right of the
  source"). A tab context-menu command activates its tab first (`CommandArg::Tab`).
- **IPC `CloseView`** never closes a window: a Home tab replaces the last view even with
  `closing_last_tab_closes_window` (only a user's Close Tab closes a window).
- **Layout values.** The GUI has no `toml` dependency (D-133), so `State` gains typed accessors
  `layout_value`/`set_layout_value` over `LayoutValue::{Bool, Float}` (an integer reads as a float; any other
  type is ignored and the default applies). The session's `[view]` level of the precedence arrives with W4b.
- **`reload_settings`** keeps the previous options or registry when a file cannot be read at all (a newer
  `schema_version`, a broken `order.toml`) and reports it as an `invalid_config_value` warning.
- **Settings at start.** `AppCore::new` reads `settings_snapshot_partial` (W4 leftovers below) and `state.toml` once, synchronously, before the
  first frame (startup, not the UI loop); every later read or write is a job or the watch thread. `Track` of an
  input is stamped on the watch thread.
- **Palette.** The window chrome (panels, borders, hover, error and warning strips, gaps) takes fixed colours of
  the scheme's base (a resolved `Scheme` has no `base`; the window passes the one of the scheme's file).
  Syntax colours use the fallback cycle by element id (named syntax styles need element names in the slice: W11
  with the editor). `[folder]` newer/different and older/unknown share an FC-031 class; the row category selects
  the element.
- **Fonts.** The folder tree font is clamped to 8…17 points (fixed row height); `use_system` = 13 points (egui's
  body size). Open text views keep their size when the theme changes; new views take the new editor size.
- **Hints.** egui's own tooltip delay is 0 (both styles); `HintTiming` (pure, frame times) decides when a hint shows
  and hides. Every hint of the window (tab captions, ×, +, toolbar drop-downs, panels, notices, dialogs, tree rows)
  goes through `ui::hint` (`response.hint(text)`), which reads the timing the window stores in the egui context
  each frame; no widget calls `on_hover_text` directly.
- **Inputs.** `input_stamp` watches `Spec::Local` only; any `stat` failure is `Missing`. The spooled copy of
  standard input (`<cache>/stdin/<n>/…`, what a command-line `-` becomes for the GUI) is a blob: `Unwatched`. Notice keys are
  `settings:<file name>`, `input:<view>:<spec>`, `job:<view>`, `font:<family>`, `startup:<text>`.
- **Debug events.** A tracing layer forwards `hikasa::*` events at debug level and above to the UI mailbox
  (`UiMsg::Debug`) only while `debug_messages` is on: an `AtomicBool` the core keeps current
  (`AppCore::set_debug_flag`), so with the option off nothing is formatted, posted or woken.
- **Log saving.** One `log_append` at a time (`CoreJob::Log`): the lines pushed meanwhile stay in the book and go
  with the next append, in order. A failed append drops its lines, stops saving (the book's `save` is off) and
  raises one warning notice `log:save` (also a log entry, not saved); the failure is not logged through tracing,
  which would feed the book it failed to save. A snapshot with changed options resumes saving.
- **Every `Service` error reaches the book (D-116):** besides the views' notes and the settings notices, a failed
  layout write (`layout`), a drop that cannot be opened (`drop`), a Home Compare that cannot be resolved (`Home`)
  and the warnings of a reload done after the window's own `options.toml` write (`settings`).
- **Tab drags across windows** use the pointer at release plus the window's `inner_rect` origin as the global
  position; `resolve_tab_drop` (pure) decides with every window's outer and tab-bar rectangles.
- **Settings writes of the window run one at a time** (a queue in `AppCore`): two quick toolbar changes would
  otherwise both pass the stamp check of `SettingsFile::save` before either writes. Each toolbar change re-reads
  `commands.toml` in its job and applies only its own edit (a `Conflict` from an outside edit is retried twice).
  The reloads of D-117 go through the same queue. Each queued job has a sequence number: a snapshot applies only
  when its job is the newest queued and newer than the last applied; an older one is kept and applied only if
  every later job fails (the window then drops its own edit that the disk did not take). So a reload of an outside
  edit that ends after the window's own edit was made never reverts that edit in memory, and the later write's
  snapshot carries both. A write that wrote `options.toml` calls `reload_settings` in its job (D-119): the window's
  own option writes reach later comparisons (and W4c's Apply needs no extra call).
- **Input watch.** `InputWatch` counts views (`track`/`untrack` per view; `retain` for an input already watched,
  which keeps its stamp). `AppCore` remembers which inputs each view tracked and untracks exactly those when it
  closes (the option may change in between).
- **Root close.** The root viewport's `CancelClose` is sent whenever `close_window` did not return `Exit`: after
  the root swap the closed window's model is gone, yet eframe must keep running for the next window.
- **Viewport builders.** A non-root window's `ViewportBuilder` is made when its viewport first shows (model
  geometry) and kept; later frames change only its title. `CloseViewport` (the root swap) drops it, so a window
  that leaves the root viewport again opens with its model geometry.
- **egui theme.** `ApplyTheme` and the start call `Context::set_theme` with the scheme's base and write the
  visuals into both of egui's styles: egui never follows the OS theme itself.
- **Keys.** `keys::shortcuts_of` gives the key as typed first, then for Ctrl with `=`/`+`, `-`, `0` the keypad
  alias (`Ctrl+Num +`, …), the first binding wins. The window root dispatches with the focused area first (a Home
  path field by its id, the text panes, the folder tree), then the root; the tree and the panes run their own
  bindings (overrides of focus-bound keys) before their scrolling. While a dialog is open the view under it takes
  no key (`keys = false`).
- **Notices of closed views** (`input:<view>:…`, `job:<view>`, any with an action on the view) go when the view
  closes (`Notices::forget_view`).
- **Tab reorder.** `move_tab` within one window takes the insertion point among the tabs before the move (the bar
  passes the slot under the pointer, `0..=len`) and shifts it past the moved tab.
- **Per frame.** The command tables are an `Arc` shared with the drawing layer; the toolbar items are cached per
  tables and scope. The menu bar is still rebuilt each frame (it depends on the check states and availability of
  the moment, and costs microseconds). Dialogs are polled once per window and frame. The log panel formats only
  the rows on screen (`ScrollArea::show_rows`).
- **Gate `two_windows` (A4.10).** Measured in two variants: the second window at the first screen
  (`two_windows`) and at the coloured screen (`two_windows_late`, `HIKASA_PERF_SECOND_WINDOW=coloured`). After the
  review fixes both fail the PSS limit under llvmpipe (402.3 and 401.4 MiB; frame work met), and so does the
  single-window `text_scroll` in the same run (396.9 MiB): the anonymous memory is unchanged (315–318 MiB, the
  second window ≈ 1–2 MiB of it), the rise is the file-backed share of Mesa's libraries (docs/02, W4a). Open: the
  maintainer's GPU run (end of W4c) decides the gate; no GUI change reduces the software renderer's mappings. *Corrected
  in W4c: the swing was the allocator, see D-134.*
- **"No more differences".** The message panel's "Do not show again" (bound to `[next_difference]
  show_message_panel`) is implemented and tested through `dismiss_message`; no view raises the "no more
  differences" message yet: `DiffNext`/`DiffPrev` are disabled when there is none, and wrap-around and the message
  belong to the next-difference work of W9/W11 (deferred).
- **Palette base.** `Palette::from_scheme_base(scheme, base)` is what the window uses (the base from the scheme's
  file); `from_scheme` takes the built-in's base, light for other names.
- **Tests.** The M1 GUI tests run through a `TestApp` wrapper over the focused window; the W4a acceptance tests are
  `src/tests_shell.rs`, the goldens `gui/hikasa-gui/tests/golden/` (`HIKASA_BLESS=1`).

**W4b**

- **Store queue.** Every read or write of a session or workspace file is a *store job* (`app/flow.rs`): one at a
  time, in order, beside the settings queue (which stays the only way to write settings files; MRU lists go through
  `state_update` like the layout). So a Save never races an auto-save of the same view, and a read of the tree runs
  last (`refresh_wanted`). A job's body receives the view's auto-saved slot **as it is when the job starts**
  (`slots`, kept per view also after its tab closed): a timer job and a close job queued together write one file.
- **`dirty` is cached.** `ViewSession::dirty` is `is_dirty(loaded, current)` computed on a job when specs or rules
  change, which only the Session Settings dialog (and later rule editors) can do; a close therefore decides on the
  UI thread with no disk access (D-105), and a description change counts as dirty too (`is_dirty` ignores it; the
  close would otherwise lose it). `ViewSession` also has `decided` (the close action ran), `autosaving` (one job per
  view) and `description`.
- **Close flow.** Every question of a close is asked **before** anything is written or closed: a Cancel on the
  third view of a workspace load leaves all three as they were, and the answers run together afterwards. A **Yes holds
  the close**: the tabs stay until every save of the flow returned. When one fails the flow ends as a Cancel (no tab
  closes, no exit, no workspace load, `decided` reset), the tab that failed is activated with the reason in its message
  line, a notice and a prompt ("The changes could not be saved, so the tab stays open"). This is the choice for every
  path, Exit included: an exit waits for the saves with the window alive (the 2 s of `flush` no longer matters to
  them), so a failed save never quits. Auto-saves and view-state writes do not hold the close (no question was
  answered). A shutdown that arrives during the wait (IPC, the end of a desktop session) still has only `flush`'s 2 s;
  it answers no question and its failure reaches the log only. Only one flow runs at a time; a second close while one
  asks or saves is ignored, and a workspace load says so in a notice. The OS close button of a window takes its top
  dialog as a Cancel and the window stays (the next press closes it; a dialog under it is kept); a flow asking in that
  window ends as a Cancel, one asking in another window is not touched. A prompt raises its window
  (`Effect::Raise`). Non-interactive closes go through `close_model`, which takes the D-120 decision for
  any view no flow decided, so no close path can skip it. An `on_exit_save_workspace_as` saved before a Cancel stays
  saved (§1.4.2 puts it first); a later exit saves again. An exit refused because a flow runs saves nothing and
  does not set `exit_saved`; a Cancel or a failed save of the exit clears it.
- **One slot record.** `AppCore::slots` (view → `AutoId`) is the only record of a view's auto-saved file: §1.4.9's
  `ViewSession::slot` is gone (`link_text` takes the slot). A view opened from an auto-saved file enters it at the open
  and has `last_autosaved` = what the file holds (so an unchanged view is not rewritten, `same_autosaved`). Entries of
  closed tabs go when the store queue is empty.
- **Auto-save and the file's identity.** The job reads the slot file and keeps its `locked`, `favorite` and `overrides`
  (`merged_with_file`): the view owns neither. Save Workspace As does the same for an inline tab with a slot. A locked
  slot file (`slot_locked`) is not written: the job reports `blocked`, the view is dirty and its close asks "Save changes
  to session?" (not in the D-84 table, which has no such row: `ViewSession::close_action` adds it before
  `close_action`); Yes writes a new auto-saved file. `loaded` is set for such a view so that the dialog can compute
  `dirty`.
- **An auto-save that became moot.** When an auto-save starts, a view that is `Named` by then is skipped; if the Save As
  landed after the tab closed the view is remembered in `named_after_close` for the same check. A failed delete of the
  slot after a good Save As is a warning notice; the view is `Named` all the same.
- **Views follow the store.** A rename or delete in the Home tree returns `StoreDone::Moved` / `Deleted` and every tab
  of every window follows (case-folded, a folder command lists its sessions first): origin, title, caption, a child's
  parent link, a pending tab's reference. A deleted session's view becomes `Unnamed` with `loaded = None`, so its D-84
  row is `AutoSave`.
- **A save never overwrites another session.** `ViewSession::seen` is the file as read at the load or as last saved.
  `queue_save` compares it with the file (`same_session`: type, specs, description, parent, rules) and refuses on a
  difference; a missing file is `not_found`. Lock, favorite, overrides, `view` and `display` may change under a view.
  So an outside rename that left a new session under the old name is refused, and so is any outside change of what the
  view owns; Save As remains the way. A change of the defaults under an open view counts as a difference too (rules
  are read over the defaults): a refusal, never an overwrite. A view-state write skips a file that went or differs,
  silently: a clean close of a stale view is no error.
- **Timer without frames.** `AppCore::autosave_wake_in` gives the seconds to the earliest timer; `step` turns it into
  `request_repaint_after`. eframe 0.37 calls `App::logic` (not `ui`) for a repaint of a hidden window, with its input,
  and so egui's time, frozen: the timer runs on `timer_now`, a monotonic clock set each step, not on `i.time`. No
  thread of its own was needed.
- **`max_auto_saved_sessions = 0`** prunes once at startup (`after_first_frame`); any change of the option prunes
  (`session_prune_autosaved`), as `autosave` itself no longer runs at 0.
- **Settings OK.** `settings_ok` marks the view `settings_pending` and, with `loaded`, `dirty` on the UI thread, so a
  close before the job's answer asks; after the answers the flow waits for the `Reopen` (the capture must hold the new
  specs). `Reopen` applies the session's `display` like an open.
- **A workspace tab's `view` table** overlays the session's before the model is built; the tab's own keys win (D-85).
- **Inline tabs** keep the explicit and unknown keys of the table they were restored from: `inline_table(session,
  previous)` (`PendingTab::inline`, `ViewSession::inline`).
- **Startup.** A workspace that does not exist is `StoreDone::StartupMissing`, a warning notice; any other failure to
  read it stays an error.
- **Save As** resolves the spelling of an existing name first (`spelled`): Replace of a case variant works.
- **`explicit_save`** is set by `SessionSave` on a locked view before its prompt and cleared by a successful save;
  answering No leaves it set, so the row *locked, explicit save* of D-84 is reached at the close.
- **Signatures.** `session_for_view(…) -> Result<(Session, Vec<Warning>), ApiError>`. `session_apply_scope(…, parent:
  Option<&str>)` takes the link's text instead of `&SessionRef`: an auto-saved parent (`_autosaved/<id>`) has no
  `SessionRef`, and D-82 allows one. New calls beyond §1.4.15: `session_load_autosaved`, `session_resolve_parent`
  (a child's effective rules, for FC-062). `Service::workspace_views` is gone (`workspace_load` replaces it). The GUI
  has no `toml` dependency, so `sessions::view_state` holds the conversions (`OpaqueTable`, `layout_table`,
  `text_display_table`, `apply_text_display`, `folder_display_table`, `apply_folder_display`, `view_bool`) and
  `State::overlay_view` the D-108 precedence.
- **`display` of a session.** Tree, Home and workspace opens restore `display` (SES-010); IPC `LoadSession` clears it
  as a script load does. `display` holds the serialized filter, `ignore_unimportant` and, for folders, the folder
  mode and `suppress_filters`; gap bars are not kept.
- **`TabDuplicate`** stays the W4a path (the same request, built at once) rather than `session_for_view` →
  `session_view_of`: a duplicate opens synchronously and §1.4.2's origin `Unnamed` holds. The result is **not** the same
  session: the comparison, display and layout are copied, the parent link, the description and the session reference
  are not (a new view of the same inputs).
- **`HomeModel`.** `left`/`right` moved to `panel`; `untouched()` replaces the comparison with `default()` (the tree
  is filled in the model). Additive to the §1.4.11 struct: `error`, `rename`, `focus_search`, `dragging`;
  `TreeNode::Heading` (*New*, *Auto-saved*) and `customized` on the *New* rows (a defaults file exists). A chosen type
  with paths of another kind is the panel's error: the kind is inferred as without a type and compared
  (`resolve_open` with a forced kind would open a failing view). Opens from Home target the Home **view**, not its
  tab index: the user may switch tabs while the job runs.
- **`SessionNewList`, `SessionOpen`.** Comparisons need specs, so New Session ▸ *type* opens a Home tab with that
  type selected; Open Session opens Home.
- **Catalogue.** `SessionHome` gains the caption "Go to Home" (an entry without one is invisible in menus), `Find` of
  `home` the caption "Find..." and its own description. `Find` stays `planned` in the file views. `WorkspaceManager`
  shows both in Load Workspace ▸ (§1.4.3) and as its own Session entry (catalogue).
- **Pending tabs.** `Tab::scope` of a pending tab comes from its type; `model()` is `None`. A tab that cannot be shown
  (a type the GUI lacks, an empty spec) is a Home tab with a warning at restore, not a pending tab; one that fails
  when activated becomes Home with the reason in its message panel. `Tab::title` gives a session's tab its caption
  (the reference) also once live.
- **IPC.** `LoadWorkspace` registers the tabs on the handler thread and answers `Views` at once; the close actions
  run in the window afterwards, so a Cancel there closes the registered views with `close_unopened`, and a workspace
  load during a running close is refused the same way.
- **Workspace shortcuts.** D-111 rule 3 is `AppCore::dispatch_workspace`; a focus-bound key (no Ctrl, Alt, Cmd) only
  at the window root. Manage Workspaces takes the shortcut as text (`Shortcut::parse`), not by key capture
  (§1.4.12).
- **Gate G12.** The first PSS comparison (workspace against `scroll`, both at the first screen) sampled while the 1M-line
  comparison ran, so its −123 MiB measured the job's progress and proved nothing: dropped. §1.4.17 now compares two
  workspaces over the same files with a small active pair at a settled point; the figures are in docs/02.

**W4c**

- **`DialogRequest`.** A dialog that writes settings hands the body to the window's ordered queue, so the §1.4.20 rules
  (one write at a time, sequence-numbered snapshots, `Acknowledge`, `reload_settings`) hold for Apply and the wizards too.
  `DialogModel` gains `take_requests() -> Vec<DialogRequest>` (`Settings{title, body}`, `Ask(Prompt)`, `Close`,
  `Refresh`), `busy()`, `size()` and `wants_keys()`; `Followup` gains `Options(ConflictAnswer)` and `WizardOverwrite`.
  The body returns the paths to acknowledge and reports its outcome to the dialog over a channel; a settings job always
  reports `Ok` to the queue, so a failed Apply is the dialog's message, not also a notice. `AppCore::poll_all` polls the
  dialogs **after** `poll_jobs`, so a dialog sees the result in the tick its job ended. The write job also calls
  `reload_settings` when it wrote a format file (an import).
- **Generic fields.** The GUI has no `toml` (D-133) and the pages are data over `key_infos`, so core gains
  `settings::form`: `FormValue`, `form_value`, `set_form_value` (by dotted path through the typed value), `clamp_value`
  and `enum_caption` (the captions are keyed by the schema's spellings, which an adapter may not tabulate: architecture
  check 7).
  `CommandOverrides` also gains `prune_view`: an override or a view left at its defaults goes, so an edit undone writes
  nothing. `reset_view` keeps the overrides of `planned` commands (D-129). Scheme elements are read and set by name
  (`SchemeFile::{colour, set_colour, flag, set_flag}`, `Scheme::{colours, flags}`, `SchemeGroup`). `SettingsSnapshot::scheme`
  moved to `theme::selected_scheme`, which the preview calls with the working copies.
- **Sparse working copies.** A colour, flag or syntax part equal to the scheme's base is stored as "not set" at the edit,
  and after each save the saved files' working copies are taken from the fresh snapshot, so a value that was dropped as a
  default does not read as an unapplied edit.
- **Preview.** `AppCore::preview` (base and working `theme.toml`) exists from the first theme edit until the dialog
  leaves its stack by any path (`dialog_closed`: Cancel, OK, a `Close` request, the OS close button). While it exists
  `reapply_theme` keeps the working scheme; `theme()` and `scheme_base()` return the preview's. One step = one
  `ApplyTheme`; a step that changes nothing emits none. The UI reports `DialogInput::Edited` each frame of the dialog,
  which costs one flag test.
- **One Options dialog** in the process: a second `ProgramOptions` raises the first (the preview has one owner).
- **Unapplied edits (§1.4.7).** A snapshot the window applies reaches the dialog (`follow`): a file without unapplied
  edits follows it silently; one with edits keeps its old stamp and raises the *Reload (discard my edits)* / *Keep my
  edits* notice, so the next Apply meets `Conflict`. The warning notice of a reset key gains *Options…* (`OpenOptions`).
- **Conflicts** are asked in one prompt for every conflicted file; Overwrite saves only those files with `force`,
  Reload reads a fresh snapshot on a job. The files that succeeded stay written; after Cancel the conflicted files keep
  their old stamp, so the next Apply asks again. After OK and Reload the dialog stays open.
- **Enter in the Options dialog** presses no button: its text fields use Enter, and Apply must not fire on a typed value
  (D-112's "Enter = default button" applies to a dialog that has one). The wizards have one.
- **Open With item.** The item editor is a panel on the page, not a second modal (one body under the same Esc and F1);
  New, Edit, Clone, Delete and the check by `split_words` and `OpenWith::validate` as specified.
- **Tree heading.** `tweaks` is a page without content (the parent of the seven sub-pages).
- **Item lists of the Export wizard** come from `package::category_files` on a job, not from `session_list`,
  `format_list` and `workspace_list`: the names of `--item` are the manifest's (`_defaults/…` included), which those lists
  do not spell. Restore adds "Session defaults" as a choice for the category `sessions` (without it a restore can not keep
  `_defaults`), and `state`, `formats` as listed. The Export overwrite test (`exists`) runs on the job.
- **`gui_import_package(&Path, launcher, policy)`** has the shape of `gui_load_workspace`; the path must be absolute
  (`classify_single_arg` makes it so; core may not read the current folder). The handler refuses a relative path with
  `invalid_request`; a package that does not validate is the wizard's step 1, not an IPC error. `hikasa-gui x.hikasa-pkg
  --silent` imports before the IPC endpoint is bound and prints only failures (exit 0 or 100). A second `ImportPackage`
  while the wizard is on step 1 changes its path; see *Review fixes* for the later steps.
- **Drops** do not reach the view while a dialog is open; a Path field of the Options dialog takes a drop when hovered or
  focused (Browse: W9).
- **Layout.** A separator in a horizontal row takes the available height, so the page row of the Options dialog has an
  explicit height (at most the window's minus 150 points) and the buttons stay on screen. The dialog tints text fields
  (`extreme_bg_color`): in a light scheme they were invisible on the dialog's background.
- **Bench `options`** needs no specs: `drive_options` runs on the core before the frame; 2 frames per page, then 60 idle
  frames, `memory_settled` sampled at the end.
- **Review fixes.**
  - *What this build does not know.* `SettingsSchema::set_aside` runs on the raw table after the migrations:
    `CommandOverrides` moves views and command ids outside the catalogue, and the override of a `planned` command with a
    shortcut that does not parse, into the kept unknown keys with a warning, in both load modes. So they are never in
    the value, `validate` and `repair` never see them, `settings set` and `settings_save` keep them, and a package with
    them imports. `settings set` of a new key under them stays refused (`names_an_entry`); `get` answers them and
    `unset` removes them (W4 leftovers below); a bad shortcut of an `implemented` command stays a repaired problem
    (D-71).
  - *`check`* runs over `changes()`, so a list element kept as it is (`app.0`) blocks only an Apply of its own file.
  - *OK.* Any error of a run clears `close_after`, so a later Overwrite of the same run cannot close over a failed file;
    the message stays until the next Apply or OK.
  - *`settings_save`* no longer fails after writing (`reread_error`; `changed_on_disk` of a delete is a `Failed` outcome).
  - *`follow`* during a job keeps the snapshot and applies it when the job ends; every save or reload also refreshes the
    files without edits, stamps included. *Reload* during a job waits for it.
  - *Scheme deletes.* An empty built-in is deleted when the snapshot holds no unknown key, or after *Reset all*, which
    also drops a stray `base`; a later edit of that scheme forgets the *Reset all*.
  - *Open With item.* The open panel counts as an edit of `openwith.toml` (its row must stay the row it was opened on);
    Apply and OK commit it or refuse with the page shown. Its shortcut is a capture field (`OpenWithPage::capturing`).
  - *Esc.* An open egui popup (`Popup::is_any_open`) keeps the key; `DialogModel::on_key` of the Options dialog asks when
    `dirty()`; `Followup::Options(ConflictAnswer::Discard)` closes. The OS close button asks the same (W4 leftovers below).
  - *Close during a job.* `close_window`, `close_window_confirmed`, `exit` and `exit_confirmed` refuse with a notice while
    `dialogs.busy()`; `Service`-side shutdowns nobody can refuse (IPC, the desktop session) go through `flush`, which
    waits for settings writes up to 60 s and logs an error past that. `close_window_now` ends the window's dialogs.
  - *Second package.* `ImportWizard::offer`: same path nothing, step 1 takes it, the result page a fresh wizard, in
    between a prompt (`Followup::SwitchPackage`), a job a notice; the wizard is found in every window.
  - *T11.* `FONT_SIZES`, `MIN_CONTRAST` and `package_path` are in core; `PlanDto::locked_kept` counts the locked sessions
    a plan keeps, also those "delete existing" would delete.
- **Gates of the wave end.** Settled samples, median of three runs: `text_scroll` 255.4, `two_windows` 262.0,
  `two_windows_late` 257.6 MiB; `options_memory` 105.4 MiB; all met, every run in docs/02 (W4c). The earlier single runs
  proved nothing: the figures were bimodal by about 25 MiB (two of three `two_windows_late` runs above the limit, median
  382.5 MiB). The cause is glibc's dynamic mmap threshold, not Mesa's mappings as the W4a note above says; D-134 fixes the
  threshold. The peak RSS stays 419–429 MiB, above 400 MB, and is not what the gate measures.

**W4 leftovers** (after 0.1.90, before W5)

- **What cannot be read.** `Service::settings_snapshot_partial` reads each file on its own: one that cannot be read
  (a newer `schema_version`, broken TOML, no access) is its defaults with the stamp `Absent` and is named in
  `SettingsSnapshot::unreadable`. The window starts with it (no file but `options.toml`, which the service needs, stops
  a start, and the readable files are no longer replaced by defaults) and the dialog's *Reload* uses it;
  `settings_snapshot` stays strict (SES-052), and so do the window's settings jobs, so with such a file each write or
  reload still ends in an error notice and the window keeps what it has. `FileOutcome::Unreadable`: `settings_save` asks
  the file on disk after a `Conflict` and before a `force`, and never writes or deletes one it cannot read; that is the
  guarantee, whatever the dialog offers. The page names the file and the reason. The prompt holds *Reload* and *Cancel*
  only and covers the unreadable files alone: a plain conflict beside them keeps its edits and meets its own prompt at the
  next Apply.
- **`force`.** `settings_save` with `force` reads the file again, then saves the value given with the unknown keys of
  the file as it is now, through `IfUnchanged` on that fresh stamp: a change in between is a `Conflict` again, not a
  loss. `SettingsFile::save(Force)` itself is unchanged; `settings set --force` has just read the file.
- **OS close.** `DialogModel::on_close`: true when the dialog stays because it asks. Only the Options dialog overrides
  it (`dirty()`); a prompt over it closes as Cancel.
- **`settings unset` of kept keys.** A key `find` reports as kept (an unknown key, a view or command outside the
  catalogue, the override of a `planned` command this build cannot read) is removed with
  `SettingsFile::forget_unknown`, with the tables that leaves empty; `old` is its value, `new` `null`. `get` of such a key
  already worked for views and commands outside the catalogue, the §1.4.20 note was wrong there; only a key inside a set
  aside override read as `null` (`find` now prefers the kept value), which also makes `settings set` of it `not_found`.
- **First-run format files.** They were written behind the watch's back, so each was `Created` at the next poll.
  An acknowledgement afterwards would race the 2 s poll and the focus poll across about twenty fsyncs, so
  `WatchCmd::MaterializeFormats` writes them on the watch thread and acknowledges what `materialize` returns (`Deferred`
  only sends the command; without a watch, the bench, it writes them itself).
- **Home and the CLI.** D-124 had no source of change but focus: a CLI write while the window is idle and unfocused was
  invisible. `Service::sessions_stamp` hashes paths, lengths and times below `sessions/` (no options, no session read);
  `WatchEvent::Sessions` calls `refresh_sessions`. The GUI's own session writes are not acknowledged, so they cause one
  extra read of the list and nothing else.
- **Dialog layout.** `size()` is the width and the height of the *body*. A right-to-left layout fills the height left, so
  the buttons were centred in the height the area remembered: they have a row of their own height now. The wizards draw
  into a body of that height (`min(height, window − 110)`), `filled` gives a list what stands under it leaves (its height
  of the last frame).
- **Moving a dialog.** The offset of the area's anchor, kept per dialog title in `egui` memory until the program ends and
  clamped so the whole dialog stays on screen. The drag area is added before the title's widgets: anything added later
  sits on top of it and keeps its clicks (the toolbar's dead buttons were the same mistake the other way round).
- **Notices and the help over a modal.** egui blocks every layer under the top modal. A notice area first shown before
  the dialog stayed under it, so *Reload (discard my edits)* of §1.4.7 was dead, and F1's panel, a panel of the window,
  could neither be scrolled nor closed. `notify::show` moves its area to the top each frame; with a dialog open the help
  is drawn after it in a foreground area on the right edge, and Esc closes the help first.
- **Tests.** `ui/tests_pointer.rs` presses and releases at a widget's place on screen, found from the text shapes of the
  frame, over the whole `HikasaApp`: tab bar, Home, status bar, notices, dialogs, wizards, menus, view panes.

### 1.5 W5 — Archives

**Ids.** VFS-002 (archive back-end: zip, tar, gz, bz2, xz, zst; nesting; temp cache; rebuild on write; type masks),
ARCH-021 (nested members), VFS-001 (capabilities per format), FC-001 (archive sides), FC-015, FC-025 (archive-provided
CRC), FC-077 and FC-072 (the VFS part: an empty archive, copying into an archive), TC-002 (archive members), OPT-010,
ARCH-101 (item results of rebuilds), CLI-000 (the new flags comply), gate G6. Decisions D-135…D-155. Out of scope: 7z
and the other P1/P2 formats, passwords and AES (P1), folder operations, confirmations and backups (W6, §1.5.17), the
VFS browse dialog (W9), editing a member in a text view (W10/W11).

**Files.** `core/hikasa-core/src/` (`spec/mod.rs`, `warning.rs`); new `core/hikasa-vfs/src/archive/` with `mod.rs`
(`ArchiveVfs`, `ArchiveConfig`), `format.rs` (`ArchiveFormat`, `ArchiveMasks`), `resolve.rs`, `name.rs` (member names),
`index.rs` (the member tree), `zip_read.rs`, `tar_walk.rs`, `codec.rs` (decoders, encoders, guards), `cache.rs`,
`time.rs`, `shared.rs` (`SharedReader`, `RangeReader`) and, W5b, `batch.rs`, `overlay.rs`, `rebuild.rs`, `zip_write.rs`, `ops/roots.rs`; `core/hikasa-vfs/src/`
(`vfs.rs`, `error.rs`, `router.rs`, `source.rs`, `memory.rs`, `local/mod.rs`, `local/windows.rs`, `ops/mod.rs`,
`ops/plan.rs`, `ops/touch.rs`, `lib.rs`); `core/hikasa-vfs/tests/` (`archive_resolve.rs`, `archive_read.rs`,
`archive_safety.rs`, `archive_fuzz.rs`, `archive_time.rs`, `archive_cache.rs`, `archive_write.rs`,
`archive_faults.rs`, `common/archives.rs`) and `benches/archive_crc_1gb.rs`; `core/hikasa-folder/src/` (`rules.rs`,
`scan.rs`, `model/node.rs`); `core/hikasa-session/src/` (`settings/archives.rs`, `api/mod.rs`, `api/compare.rs`,
`api/info.rs`, `api/vfs_ops.rs`, `api/vocab.rs`, `api/gui.rs`, `api/human.rs`, `api/settings.rs`);
`cli/hikasa-cli/src/` (`args.rs`, `cmd/folder.rs`), `cli/hikasa-cli/tests/` (`cli_archive.rs`, `perf_gates.rs`,
goldens); `gui/hikasa-gui/src/` (`ui/folder.rs`, `folder_view.rs`, the icon table); workspace `Cargo.toml`,
`Cargo.lock`, `core/hikasa-vfs/Cargo.toml`, `deny.toml`, `core/hikasa-session/tests/architecture.rs`,
`packaging/build-linux.sh`; `help/archives.md` (new), `help/vfs.md`, `help/folder-compare.md`, `help/settings.md`,
`help/index.md`.

#### 1.5.1 Parts (D-135)

W5 is one wave in two sequential parts, one implementer each; a part starts when the previous one is committed on
`feat/m2` and ends with the tree green.

| Part | Content | Sections | Acceptance |
|---|---|---|---|
| **W5a Read** | spec model, resolution, `ArchiveVfs` reads for every format, index, cache, safety, time, CRC, FC-015 scan, settings, every read command of the CLI, the GUI's rows, dependencies, G6 | §1.5.2–§1.5.9, §1.5.11–§1.5.16 | A5.1…A5.13, A5.19, A5.20 |
| **W5b Write** | batches, overlay, rebuild, `create_archive`, the write capabilities, `vfs cp x a.zip/dir/` and the other write commands on members | §1.5.10 | A5.14…A5.18, A5.20 |

At the end of W5a every primitive on a member answers `OperationUnsupported` (the W3 default bodies) and no write bit
is reported, so a read-only tree ships if W5b waits. Copies **out of** an archive work in W5a: they only read it. The cut
is natural: W5b only adds code behind the capability bits.

#### 1.5.2 Spec model (D-137)

`Spec::parse` does not change: it never produces `ArchiveMember`, so ARCH-020's round trip stays as it is. Members
appear only through resolution (§1.5.4), `Vfs::child` and `Spec::archive_member`.

- **Root.** `ArchiveMember { archive, member: "" }` is the archive **as a folder**. Its canonical form keeps the existing
  `Display`: `/x/a.zip/`, with the trailing `/` that tells it from the file `/x/a.zip`.
- **Member text.** `member` is `/`-separated on every platform, has no leading, trailing or doubled `/`, and no `.` or
  `..` component. Nesting wraps: `a.zip/b.tar.gz/c.txt` is `ArchiveMember { archive: ArchiveMember { archive: a.zip,
  member: "b.tar.gz" }, member: "c.txt" }`. On Windows the canonical form is `C:\x\a.zip/inner/x`; parsing it again
  gives the local path `C:\x\a.zip\inner\x`, which resolution maps back.
- **Round trip (P-ARCH-1).** For every archive spec `s` on a given tree, `resolve_spec(parse(canonical(s)), use)` = `s`,
  with `use = Folder` when `s` is a root or an input with a trailing separator, else `Any`.

```rust
// hikasa_core::spec (additive)
impl Spec {
    /// `ArchiveMember { archive, member }`; `member` is stored as given, `""` is the root.
    pub fn archive_member(archive: Spec, member: impl Into<String>) -> Spec;
    /// The spec of the archive that directly holds this member (`None` for anything but `ArchiveMember`).
    pub fn container(&self) -> Option<&Spec>;
    /// The file outside every archive (`a.zip` for `a.zip/b.tar/c`); `None` for anything but `ArchiveMember`.
    pub fn outermost_file(&self) -> Option<&Spec>;
    /// Levels of archives around this spec: 0 for a local path, 2 for `a.zip/b.tar/c`.
    pub fn archive_depth(&self) -> u32;
}
```

`Spec::parent` (W3 said W5 adds members) and `Spec::file_name`:

| Spec | `parent()` | `file_name()` |
|---|---|---|
| member `"x/y"` of `A` | member `"x"` of `A` | `y` |
| member `"x"` of `A` | root of `A` | `x` |
| root of `A` | `A.parent()` (a folder outside, or for a nested `A` the folder holding it inside its own archive) | the name of `A` |

`ops::check_item` refuses a root like a file-system root: `InvalidTarget "is a root folder"` (an archive is removed as
the file it is, `rm a.zip`, never through `rm a.zip/`).

#### 1.5.3 `ArchiveVfs`, formats, capabilities, router (D-136)

`ArchiveVfs` is a `Vfs` **stacked on another `Vfs`**: it serves `ArchiveMember` specs itself and passes every other spec
to its base, and it reads containers only through the base (`open_read_with`, `stat`), so `MemoryVfs` tests every
algorithm, faults included, and no back-end is special. `hikasa-vfs` cannot name `hikasa-session`, so the masks reach it
as a value.

```rust
// hikasa_vfs::archive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ArchiveFormat { Zip, Tar, TarGz, TarBz2, TarXz, TarZst, Gz, Bz2, Xz, Zst }
impl ArchiveFormat {
    pub const ORDER: [ArchiveFormat; 10];            // the same order as ArchiveType::ORDER
    pub const fn writable(self) -> bool;             // Zip and the five tar formats (D-63)
    pub const fn is_single_stream(self) -> bool;     // Gz, Bz2, Xz, Zst
}
pub struct ArchiveMasks { /* (format, compiled MaskList) in ORDER; an empty mask is left out */ }
impl ArchiveMasks {
    pub fn new(masks: impl IntoIterator<Item = (ArchiveFormat, String)>) -> Result<ArchiveMasks, MaskError>;
    /// The first format in ORDER whose mask matches `name`, case-insensitively (as `ArchiveTypes::detect`).
    pub fn detect(&self, name: &str) -> Option<ArchiveFormat>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits { pub entries_max: u64, pub stream_max: u64, pub cache_max: u64, pub ratio_max: u32,
                           pub ratio_floor: u64, pub nesting_max: u32, pub paths_max: u64,
                           pub depth_max: u32 }                            // Default: §1.5.7 table
pub struct ArchiveConfig {
    pub masks: ArchiveMasks,
    pub limits: ArchiveLimits,
    /// "Now" with the user's zone (T1): the zone of DOS times, the time of a written member. The session passes
    /// its injected clock.
    pub clock: Arc<dyn Fn() -> jiff::Zoned + Send + Sync>,
    /// The folder under which the per-session cache folder lives; created on the first write only.
    pub cache_root: PathBuf,
}
pub struct ArchiveVfs { /* base, config, indexes, cache */ }
impl ArchiveVfs {
    pub fn new(base: Arc<dyn Vfs>, config: ArchiveConfig) -> ArchiveVfs;
    /// Masks and limits changed (`Service::reload_settings`): later calls use them, running calls keep theirs.
    pub fn set_config(&self, config: ArchiveConfig);
    /// Removes the session's cache folder (best effort; also on `Drop`).
    pub fn cleanup(&self);
}
impl Vfs for ArchiveVfs { /* … */ }

// hikasa_vfs::vfs (additive; every body is defaulted, so implementors compile unchanged)
pub trait Vfs: Send + Sync {
    // … M1 and W3 methods …
    /// Does the slow part of reaching `spec` now, with cancellation and progress: for a spec inside an archive, the
    /// index of every archive around it (D-139). Default: nothing.
    fn prepare(&self, _spec: &Spec, _ctl: &Ctl<'_>) -> Result<(), VfsError> { Ok(()) }
    /// `prepare`, then the members of the archive that directly holds `spec`, synthesized folders included; `None` outside
    /// an archive. A scan budgets by it (§1.5.11). Default: `None`.
    fn archive_entries(&self, _spec: &Spec, _ctl: &Ctl<'_>) -> Result<Option<u64>, VfsError> { Ok(None) }
    /// `open_read` that can be cancelled while an archive member is decoded (`Cancelled`). Default: `open_read`.
    fn open_read_with(&self, spec: &Spec, _cancel: &CancelToken) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.open_read(spec)
    }
    /// The archive format of a file name per the masks; `None` on back-ends without archives.
    fn archive_format(&self, _name: &str) -> Option<ArchiveFormat> { None }
    /// Warnings raised since the last call (D-145). Default: none.
    fn take_warnings(&self) -> Vec<Warning> { Vec::new() }
}
```

`VfsSource` gains `with_cancel(self, &'a CancelToken) -> Self` (default: never cancelled); its `open` calls
`open_read_with`, and `read_all` passes its own token. The scanner and `quick` set the job's token.

**Formats.** A format is decided by the mask of the file name, never by content (the content can only confirm it, §1.5.7
rule 8). A single-stream format is an archive of **one member**, named after the container without its last `.ext`
(`x.txt.gz` → `x.txt`; a name without a dot → `<name>.data`; a stored name in a gzip header is untrusted and ignored).

| Format | Mask type | Container | Member reads (§1.5.5) | Write (W5b) |
|---|---|---|---|---|
| `zip` | `zip` | central directory, random access | decode one member | rebuild |
| `tar` | `tar` | one pass over the file | range of the file | rebuild |
| `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` | `tar_*` | one pass over the decoded stream | spool, then the whole stream in the cache | rebuild |
| `gz`, `bz2`, `xz`, `zst` | `gz` … | one pass counts length and CRC | spool the stream | none: `OperationUnsupported` |

Concatenated streams are read to the end (`MultiGzDecoder`, `MultiBzDecoder`, xz multiple streams, zstd frames).

**Capabilities** (ARCH-030). Common set `C` = `READ LIST SEEK MTIME CASE_SENSITIVE`. Never reported: `CTIME`, `LOCAL_PATH`,
`TRASH`, `SET_CREATED`. `Vfs::capabilities` needs only the masks: it does not open the archive. A member of a nested
archive gets the bits of the innermost format, and the write bits only when every archive around it is writable.

| Format | W5a | W5b adds (`W` = `WRITE MKDIR REMOVE RENAME SET_TIMES SET_ATTRS CREATE_SYMLINKS`) |
|---|---|---|
| `zip` | `C CRC_PROVIDED DOS_ATTRS UNIX_MODE SYMLINKS` | `W` (`set_attrs`: `Mode`, and `Dos` with the `R` bit only) |
| `tar`, `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` | `C UNIX_MODE UNIX_OWNER SYMLINKS` | `W` (`set_attrs`: `Mode` only) |
| `gz`, `bz2`, `xz`, `zst` | `C` | none |

Each zip entry carries one attribute family: `Attrs::Unix { mode, uid: 0, gid: 0 }` when the entry has a Unix mode, else
`Attrs::Dos`; the quick tests compare only two entries of one family, as in M1. Tar entries are `Attrs::Unix`.
`Entry::file_id` is `None`. Symbolic links are reported, and `FollowLinks::Yes` never follows one: the entry stays a
`Symlink`, `followed = false`, and no `EntryError` is added (§1.5.7 rule 5).

**Router.** `VfsRouter::with_archives(self, Arc<ArchiveVfs>) -> VfsRouter` (additive). `ArchiveVfs` is built on the same
`LocalVfs`; `Service::rebuild_router` keeps the `Arc`. Extends the tables of `router.rs` and §1.3.1:

| Spec | Read methods, `child`, `capabilities`, primitives, `prepare`, `open_read_with` | `rename(from, to)` | `copy_file_fast` | `local_path` |
|---|---|---|---|---|
| `ArchiveMember`, archives set | `ArchiveVfs` | both inside one archive → `ArchiveVfs`; two archives → `CrossDevice` (D-97 expands it); mixed kinds → `OperationUnsupported` | `Ok(None)` | `None` |
| `ArchiveMember`, no archives (M1 embedders) | `Unsupported { what: "archive specs" }` (M1 row) | same | `Ok(None)` | `None` |
| every other spec | unchanged | unchanged | unchanged | unchanged |

`Vfs::archive_format` and `take_warnings` of the router ask the `ArchiveVfs`. `child(dir, name)` of an archive spec joins
`name` to `member`; a `name` that fails §1.5.7 rule 1, or a lossy name from a disk, whose text does not say what it is, is
`InvalidName`. A lossy name a listing of this archive gave is the member's key, so it joins.

**Stamps.** An archive is identified by its key: the canonical spec of the container plus the stamp (length, modification
time) from `base.stat(.., Yes)`; a nested archive adds the stamp of the outermost file and its member position. Every
`ArchiveVfs` call re-stats the outermost file once; another stamp drops the index and its cache files. A same-length rewrite
within the file system's time resolution is not seen (accepted, as `InputStamp`, D-118).

#### 1.5.4 Spec resolution (normative; D-138)

Maps the local path of a typed spec to the spec that names the item. It lives once in `hikasa_vfs::archive::resolve_spec`
and uses **only** the `Vfs` passed to it (`stat`, `archive_format`, `prepare`), never the OS: no listing, no current folder,
no environment. It is deterministic: the same tree gives the same answer on any thread, and it never fails (a problem is
reported when the spec is used).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecUse {
    /// The named item, whatever it is: an archive file stays a file.
    Any,
    /// A folder is wanted: an archive file is its root (`folder compare`, `vfs ls`, an input ending in a separator).
    Folder,
}
pub fn resolve_spec(vfs: &dyn Vfs, spec: Spec, use_: SpecUse, cancel: &CancelToken) -> Result<Spec, Cancelled>;

// The pure core, tested on every platform for both path styles with a table-driven fake.
pub trait Probe {
    fn kind(&self, spec: &Spec) -> Probed;                  // Dir | File | Other | Missing | Failed
    fn format(&self, name: &str) -> Option<ArchiveFormat>;
    fn open(&self, archive: &Spec) -> bool;                 // `prepare`: false when the archive cannot be read
}
pub fn resolve_with(probe: &dyn Probe, spec: Spec, use_: SpecUse, style: PathStyle) -> Spec;
```

```text
resolve(spec, use):
 1. spec is not Local                                → spec, unchanged
 2. k = kind(spec) (links followed)
    Dir                                              → spec                 (a real folder wins, whatever its name)
    File, use = Folder, format(name) is Some         → root of spec
    File or Other                                    → spec
    Failed (permission, I/O)                         → spec                 (the error comes at use)
    Missing                                          → step 3
 3. P = parent of spec, rest = [last name]; while P exists:
      kind(P):  Missing → push P's name in front of rest; P = parent of P; continue
                Dir, Other, Failed                   → spec                 (nothing here is an archive)
                File, format(name of P) is None      → spec
                File, format(name of P) is Some      → descend(P, rest, use)
    no P left                                        → spec
 4. descend(A, rest, use):
      rest holds "." or ".." or an empty name        → spec of step 1       (never a member; fails at use)
      no name but the last matches a mask, and (use = Any or the last does not match)
                                                     → archive_member(A, rest joined by "/")    (A is not opened)
      open A (prepare); on failure                   → archive_member(A, rest joined by "/")
      walk m over the prefixes of rest:
        member m Missing                             → archive_member(A, rest joined)   (a missing member, or a target)
        member m is Dir                              → next prefix
        member m is File, format(its name) is Some:
           names remain                              → descend(archive_member(A, m), names left, use)
           no name remains, use = Folder             → root of archive_member(A, m)
           otherwise                                 → archive_member(A, rest joined)
        member m is anything else                    → archive_member(A, rest joined)
      every prefix walked                            → archive_member(A, rest joined)
```

- At most one `stat` per path component, plus one member lookup per component inside archives. In the common case
  (`a.tar.gz/x.txt`) resolution ends without opening the archive. A test counts the calls (`MemoryVfs::stat_calls`).
- `NotADirectory` and `NotFound` are both `Missing`, so a file used as a folder behaves alike on every platform.
- Parents and names come from `LocalPath`/`Spec::parent`; `resolve_with` takes the path style so that Windows drive
  and UNC rows run on Linux too. `..` stays in a Unix path (D-06; `Path::file_name` gives no name for it, which counts as
  `..`), so step 4 refuses it: a spec can never walk out of an archive.
- Mask matching is case-insensitive on every platform (`ArchiveTypes::detect`); **member names are case-sensitive on
  every platform** (archives hold bytes, the capability `CASE_SENSITIVE` says so); the prefix on disk follows the file
  system. A type whose mask is empty does not exist for resolution.
- Resolution of a nested spec can index the outer archive before a job exists; that index is bounded by the limits but
  not cancellable. Service calls that run inside a job pass its token.

| # | Input (`/w` as below) | Use | Result | At use |
|---|---|---|---|---|
| 1 | `/w/a.zip` | Any | `Local` | the file |
| 2 | `/w/a.zip` | Folder | root of `a.zip` | listing |
| 3 | `/w/a.zip/` (trailing separator) | Folder (from the input) | root of `a.zip` | listing |
| 4 | `/w/a.zip/inner/x` | Any | `a.zip` + `inner/x` | the member |
| 5 | `/w/a.zip/inner` | Any | `a.zip` + `inner` | a folder |
| 6 | `/w/a.zip/missing` | Any | `a.zip` + `missing` | `NotFound`, or a write target |
| 7 | `/w/a.zip/b.tar.gz/c.txt` | Any | (`a.zip` + `b.tar.gz`) + `c.txt` | the nested member |
| 8 | `/w/a.zip/b.tar.gz` | Any | `a.zip` + `b.tar.gz` | a file |
| 9 | `/w/a.zip/b.tar.gz` | Folder | root of (`a.zip` + `b.tar.gz`) | listing |
| 10 | `/w/dir.zip/y` (`dir.zip` is a real folder) | Any | `Local` | the file |
| 11 | `/w/dir.zip` | Folder | `Local` | a folder |
| 12 | `/w/fake.zip/x` (`fake.zip` holds text) | Any | `fake.zip` + `x` | `ArchiveCorrupt`, not `NotFound` |
| 13 | `/w/fake.zip` | Any | `Local` | the file |
| 14 | `/w/fake.zip` | Folder | root of `fake.zip` | `ArchiveCorrupt` |
| 15 | `/w/plain.txt/x` | Any | `Local` | `NotADirectory` |
| 16 | `/w/up.ZIP/x` | Any | `up.ZIP` + `x` | the member (masks ignore case) |
| 17 | `/w/a.zip/INNER/x` | Any | `a.zip` + `INNER/x` | `NotFound` (members keep case) |
| 18 | `/w/none.zip/x` | Any | `Local` | `NotFound` |
| 19 | `/w/a.zip/../plain.txt` (Unix) | Any | `Local` | `NotADirectory` |
| 20 | `/w/a.zip/x`, zip mask empty | Any | `Local` | `NotADirectory` |
| 21 | `/w/link.zip/x` (link to `a.zip`) | Any | `link.zip` + `x` | the member (a link on disk is followed; a write replaces its target) |
| 22 | `/w/a.zip/lnk/x` (`lnk` a link member) | Any | `a.zip` + `lnk/x` | `NotADirectory` (never followed) |
| 23 | more nested archives than `nesting_max` | Any | nests one level past the limit, the rest as member text | `ArchiveLimit` |
| 24 | `C:\w\a.zip\inner\x` (Windows) | Any | `C:\w\a.zip` + `inner/x`, canonical `C:\w\a.zip/inner/x` | the member |
| 25 | `\\srv\share\a.zip\x` (Windows) | Any | `\\srv\share\a.zip` + `x` | the member |
| 26 | `C:\w\a.zip\` (Windows) | Folder | root of `C:\w\a.zip` | listing |
| 27 | `-`, `clipboard://`, `ftp://h/a.zip/x` | any | unchanged | as before |
| 28 | `/w/x.tar.gz/y` | Any | `x.tar.gz` + `y`, the archive not opened | the member |

#### 1.5.5 Index and reads (D-139)

An archive is opened once per stamp: one `Arc<Index>` behind a per-archive lock, built on first need; other threads wait
for it. An index holds the validated member tree (`index.rs`: nodes sorted by path, a map from path to node, children as
ranges), counts and warnings. **Memory.** The bound is on the whole index, and it holds on hostile input: a node and a
record take 200 bytes per entry, and a member's text 16 bytes of header, and all text together (paths, refused names, link targets, and the paths of folders
that only exist through their members, which are hashed and compared though not stored) stops at `paths_max`, so an index
costs at most `entries_max × 216 bytes + paths_max`, 462 MiB with the default limits, which is below 512 bytes for each
of the 1,000,000 entries allowed. A small archive of long names costs more than 512 bytes **per member**: the budget of
512 bytes per member is what an archive with paths under about 296 bytes meets, and what a hostile one meets only as this
total. A folder that no member names stores no text: it shares a member's text, of which it is a prefix, so nothing costs
entries × depth × path bytes. The zip crate holds the central directory beside this: the reader of an index pass stops at
`paths_max` + 128 bytes × `entries_max` of it (384 MiB at the defaults), since the crate follows the entry count whatever size
the end record states. Building an index peaks above what it keeps, the members as read beside the nodes: 696 MiB of RSS
measured for 995,000 paths of about 250 bytes in a `tar.gz` of 26 MiB, the worst shape the default limits allow; listing all of
them adds the entries (1.45 GiB in all). Indexes are
kept while their stamp holds; when the members of all indexes, counted as they are built, pass 10,000,000 the least
recently used indexes that no call holds are dropped.

| Format | Index | Time |
|---|---|---|
| zip | `zip::ZipArchive::new`, then `by_index_data(i)` for names, sizes, CRC, method, flags, attributes; the 0x5455 and 0x000a extra fields come from one read of the central directory by the pure `central_times` of `zip_read.rs` (keyed by `central_header_start`, bounds-checked, a malformed field is ignored) | 14 ms + 1 ms for 100,000 entries, measured; `by_index_raw` per entry would read every local header (268 ms warm, far worse cold), so it is not used |
| tar family | `tar_walk.rs`: a loop over the (decoded) stream reading 512-byte blocks; **the `tar` crate's `Archive` and `Entries` are not used to read**, only `tar::Header` (checksum, type, size, path, link, numeric forms) and `tar::PaxExtensions`, because `Entries` reads extension headers into memory without a limit and borrows the archive. Every member's bytes pass through `crc32fast` and are dropped | one decode of the stream |
| single stream | one pass that counts and hashes | one decode of the stream |

The walker records for each member the offset of its first block (extension headers included), of its data, and the
end of its padded data. Block rules: an all-zero block or the end of the stream at a block boundary ends the archive; a
wrong checksum or a short block is damage (§1.5.7 rule 9); `x` and `L`/`K` bodies apply to the next member (at most 1 MiB
each, else damage; a `path`, `linkpath`, `size`, `mtime` PAX record overrides the header); `g` and GNU volume labels are
skipped; GNU sparse headers walk their extension blocks. PAX `size` wins over the header, so no two readers disagree.

**Content reads.** `open_read` needs `Seek`, and a `ZipFile` borrows its archive (no self-reference in safe Rust), so
content is never a live decoder:

| Format | `open_read_with(member)` | Notes |
|---|---|---|
| zip | clone the `ZipArchive` (`Arc` metadata) over a `SharedReader`, `by_index`, copy through the guards: a member of at most 1 MiB into memory (`Cursor`), a larger one into a cache file (`File`), reused by key | `SharedReader` = `Arc<Mutex<Box<dyn ReadSeek>>>` + its own position, so threads decode one archive in parallel; the crate checks the CRC at the end |
| tar | `RangeReader` over `base.open_read_with(container)` | true random access; nothing extracted |
| tar.gz … | first member read: decode forward, drop bytes up to the member, spool only that member (memory or cache file). A second distinct member: decode the whole stream once to one cache file when `stream length ≤ free cache`, then every member is a `RangeReader` on it; when it does not fit, each read spools its own member | a single `decode_range(start, len, sink)`; one decoder per stream at a time. Listing and CRC tests never reach this path. Member by member a stream that does not fit is decoded from its start for each read: slow, and within the limits, since rule 12 binds one pass |
| gz … | spool the stream (memory or cache file), reused | |

- `Dir` → `IsADirectory`; `Symlink`, `Other` and sparse members → `OperationUnsupported` (op `open`).
- A zip member's own limits apply before decoding (§1.5.7).
- `stat`/`list` of a member open the index; `prepare` builds it with cancellation and `Phase::Scanning` progress in bytes.
  `list` passes its `cancel`. Cancellation is checked at least every 1 MiB of decoded output (G4), on every read of a zip's
  directory and in every loop and sort of `build_tree`; a cancelled spool removes its file. `stat` has no token, so a
  caller inside a job calls `prepare` first: `load_base`, `quick`, `text compare`, `vfs stat`, `vfs cat` and the sources of
  `cp`/`mv` do; `folder_prepare_in` passes the job to resolution. A call that waits for another call's index or decoder
  stops with its own token within a step of 2 ms, and the other call goes on.
- **Nesting.** A nested archive's bytes are its member's content: memory or cache file, read through the same paths.
  `depth > nesting_max` → `ArchiveLimit`.

| Entry field | zip | tar family | single stream |
|---|---|---|---|
| `kind` | `Dir` (name ends in `/`), `Symlink` (Unix mode), else `File`; other Unix types `Other` | by type: `0 7 \0` file (a name ending in `/` is a folder), `5 D` folder, `2` link, `1` hard link (§1.5.7 rule 6), `3 4 6` `Other`, `S` sparse | `File` |
| `size` | central directory | header or PAX | counted |
| `mtime`, `ctime` | §1.5.8 | §1.5.8 | §1.5.8 |
| `crc32` | §1.5.9 | computed | computed |
| `link_target` | the entry's content, at most 4096 bytes, read at index time | header or PAX, at most 4096 bytes; a longer target is not kept and the link shows none | — |

Folders that only exist through their members are synthesized: `Dir`, `mtime: None`. The root's entry: the container's name,
`Dir`, size 0, the container's `mtime`.

#### 1.5.6 Temp cache (D-140)

`cache.rs` owns every file W5 writes outside a rebuild target, and it is the only part of `archive/` that calls `std::fs`.

- **Location (T1).** `ConfigLocation::cache_dir` + `archives`: the host decides it from `EnvFacts` (`~/.cache/hikasa/archives`,
  `~/Library/Caches/Hikasa/archives`, `%LOCALAPPDATA%\Hikasa\cache\archives`, `<config>/cache/archives` when portable,
  `temp_dir` only when the home is unknown). Not the temp folder first: `/tmp` is often a RAM disk. Core reads no
  environment; `ArchiveConfig::cache_root` carries the path.
- **Owner.** On Unix `root` and its parent must belong to the running user or to the system (the owner of the lock file
  just created names the user, so no call asks for an id); otherwise the spool fails with `PermissionDenied`. The fallback
  root lies in the shared temp folder, where another user could have made it first and could swap what is in it. It also
  refuses a root process whose `HOME` still names a user's folder, which would leave root's files there.
- **Layout.** `<root>/s<n>.lock` and `<root>/s<n>/` (Unix mode 0700), `n` a random 64-bit number from the operating system
  (`RandomState`), so a session number never comes back and a dying path can never be someone else's. Files inside have
  numeric names only (`m<k>`, staged writes `w<k>`): **no member name ever reaches a path**, so traversal, twins and
  reserved names cannot touch the disk through the cache. Created on the first file; a command that only lists creates
  nothing (§2.22 of M1).
- **Identity.** Each file starts with a 16-byte stamp, the session's number and the file's, not counted in `used`. A reader
  opens the path, checks stamp and length on that handle, and reads only that handle, so nothing that later happens to
  the path reaches it. A file that fails the check leaves the table and the read decodes again: a wrong file costs a
  decode and never serves another file's bytes.
- **Session and crash.** The lock file is created with `create_new`, locked with `File::try_lock` and held for the life of
  the `ArchiveVfs`. After `try_lock` the handle and the path are compared (`same-file`): a lock counts only for the file
  that is at the path, so a handle opened before a sweep unlinked that file proves nothing. A session claims its number,
  then sweeps, then creates its folder: for every other `s*.lock` whose `try_lock` succeeds and whose file is at its path,
  the owner is gone, so its folder and then the lock file's name are removed **while the lock is held**. The OS releases a
  dead process's lock on every platform, so a crash leaves a folder only until the next session that extracts. A lock that
  fails with any error counts as held. No process ids, no clock. A folder that cannot be removed keeps its lock file, so
  a later sweep finds it. **Windows:** Rust opens files with `FILE_SHARE_DELETE`, so an open lock file can be unlinked as
  on Unix; where the file system only marks it for deletion the name refuses new opens, which the comparison reads as
  gone, and a random `n` never asks for that name again. `LockFileEx` belongs to the handle, as `flock` belongs to the open
  file on Linux and macOS.
- **Exit.** `ArchiveVfs::cleanup` and `Drop` remove the folder and the lock file in the same order, then `root` and the
  folders above it that this session made, when they are empty. `Service::shutdown` (new) calls it, and the adapters call
  it before `main` returns. A file still open on Windows may survive until the next sweep.
- **Accounting.** `used` is the sum of the cached bytes. `reserve(n)` runs before a known length is written, and every 1 MiB
  while a stream of unknown length is written. Over `cache_max`: least recently used files that no reader holds are
  evicted (each file is an `Arc`); when it still does not fit → `ArchiveLimit { limit: "cache" }`, the partial file is
  removed. A failed write (disk full) removes the partial file and reports the I/O error. A file is entered in the
  table only when complete.
- **Reuse and invalidation.** Key = archive key + member path (or the stream). A hit needs no decoding. A new stamp drops
  every file of the old key. Member reads through memory are not cached.

#### 1.5.7 Safety (normative; D-141…D-145)

Each rule is a table test (§6.5). Archives are hostile input. **A refused member** is left out of the tree and reported:
in the root's `Listing::errors` under the unique name `<refused-N>` (`N` = 1… in archive order; at most 1,000 rows), kind
`invalid_data`, message `refused member "<escaped raw name>": <reason>`; plus one `archive_member_refused` warning per
index with the total. Nothing can address it, and a rebuild keeps it untouched.

| # | Subject | Rule |
|---|---|---|
| 1 | Names | `name.rs`, on raw bytes, before anything else. Zip: `\` is read as `/` (legacy tools), then the rules; tar: a `\` refuses the member. Refused: NUL (`nul`); a leading `/` (`absolute`), so also `\\srv\x` and `\x` in zip; a first component `X:` with one ASCII letter (`drive`: `C:\x`, `C:/x`, `c:x`); a `..` component (`traversal`); over 4,096 bytes (`too_long`). `.` and empty components are dropped, and the drive check runs on what stays (`./C:/x`); a folder or an empty file whose name reduces to nothing (`./`) only gives the root its metadata; a file with data so named is refused (`empty`). A GNU long name loses its trailing NULs only: one inside stays, so `nul` refuses it. Text = UTF-8 flag or valid UTF-8 as it is; zip without the flag as the crate decodes it (CP437); otherwise lossy with the raw bytes kept for a rebuild: `name_lossy` says so, and the name is the member's key |
| 2 | Tree | A path that is both a file and a parent (`a` and `a/b`): the folder stays, the file `a` is refused (`conflict`). A member below a link member (`lnk` and `lnk/x`) is refused (`conflict`). The result does not depend on archive order |
| 3 | Duplicates | The same path twice: the last one wins (zip: the last of the central directory, `has_duplicate_names`; tar: the last in the stream), one `archive_duplicate_member` warning per index. A rebuild gives a shadowed record its winner's fate (D-156): kept while the winner is kept or touched, moved with it, to another archive too, gone with it when it is removed or replaced, with a warning. Two names that are equal only after a lossy decode are two members: the last stays and the others are refused (`conflict`), so a rebuild keeps them |
| 4 | Case twins | `A.txt` and `a.txt` are two members. Cache names are numeric, so nothing collides. Copied out to a case-insensitive disk the second meets the overwrite policy (D-96, default `fail`) |
| 5 | Links | A link member is listed with its target text and **never followed**: no read through it, no lookup through it, `stat(Yes)` returns the link. It is never materialized: not in the cache, and a copy **out of** an archive skips it (`SkipReason::Symlink`, one `archive_symlink_skipped` warning per operation), whatever `LinkPolicy` says (revises D-94 rule 6 for archive sources; `plan_copy` decides from the source's and the target's scheme). Between archives it stays a link member |
| 6 | Hard links | `1` entries become the member they name, which must be a regular member earlier in the archive, addressed by a name that passes rule 1, and must stay in the tree (not refused by rule 2, also through another link); otherwise refused (`hard_link`). A `1` entry has no data of its own whatever its size field says, unless a PAX `size` states it, so it cannot hide the next member |
| 7 | Devices, FIFOs, sparse | Devices and FIFOs are `Other`: listed, never opened, skipped by copies (D-96). Sparse members are listed; content → `OperationUnsupported`; a rebuild keeps their blocks |
| 8 | Not an archive | A mask match with other content: `ArchiveCorrupt`. Only `list`, `stat` and reads of its members meet it; the file itself stays a normal file. A zero-length `.tar` is an empty archive; a zero-length `.zip` or stream is `ArchiveCorrupt` |
| 9 | Corrupt or truncated | Never a panic (`clippy::unwrap_used` already denies them; every decoder and index pass runs inside `catch_unwind`, a panic becomes `ArchiveCorrupt { reason: "decoder failed" }`). Tar family and streams: members complete before the damage stay readable, and the root's listing gains a row `<damaged>`, kind `invalid_data`, with the offset (`Index::damage`). NUL bytes after the last gzip or bzip2 member are padding, as `gzip` reads them: accepted, with one `archive_trailing_zeros` warning per index; anything else there is damage. Zip: no central directory → `ArchiveCorrupt`; a member whose local data is bad fails alone. A rebuild refuses a damaged archive (`ArchiveCorrupt`) |
| 10 | Declared vs. streamed | Every copy reads at most declared + 1 bytes; streamed ≠ declared → `ArchiveCorrupt { reason: "size mismatch" }`, the partial output removed. The zip crate's CRC failure is `ArchiveCorrupt { reason: "CRC mismatch" }` |
| 11 | Ratio | `output / input > ratio_max` once `output > ratio_floor`, checked each 1 MiB of output on every stream and member (input is the compressed bytes consumed). For zip also before decoding, over the whole archive: declared total / container length, so an entry that declares an absurd size blocks every read of its archive; listing it still works, and `ratio_max = 0` lets the sound members be read. `ArchiveLimit { limit: "ratio" }`. `ratio_max = 0` turns the check off |
| 12 | Sizes | Decoded bytes of one pass > `stream_max` → `ArchiveLimit { limit: "stream" }`; a compressed tar's index is such a pass. A plain tar decodes nothing, so its index does not count. Zip members whose data overlap another's (known at their first read from the local header): their decoded bytes add up per index against the same limit, so the bombs of overlapping entries stop. The first member read from a range is free and sound data is never counted: a sound archive stays readable however often it is read, and a stream past the free cache may be decoded from its start for each member |
| 13 | Entries, text, depth | Members of one archive plus the folders that only exist through them > `entries_max` → `ArchiveLimit { limit: "entries" }`; the text of one index > `paths_max` → `ArchiveLimit { limit: "paths" }`, also for the central directory the zip crate reads (§1.5.5); a member with more than `depth_max` folders above it → `ArchiveLimit { limit: "depth" }`. All while indexing, on what is read, so memory stops there, and the limit fails the archive, never the process |
| 14 | Nesting | `depth > nesting_max` → `ArchiveLimit { limit: "nesting" }`; depth counts archives around the spec. A self-containing or recursive archive can do no more |
| 15 | Decoder memory | xz: `new_mem_limit` of 1 GiB; zstd: `window_log_max(30)`; a stream that asks for more is `ArchiveCorrupt` |
| 16 | Encrypted | A member with the encryption flag (ZipCrypto or AES; AES needs no crate to be seen) keeps its name, size and time, `crc32: None` (an AE-2 CRC is 0), and `open_read` → `ArchiveEncrypted`. Reported once per index as `archive_encrypted`. No password is asked, tried or guessed; a content test gives an error row. A rebuild copies it raw. An unknown method (PPMd, Shrink …) → `Unsupported { what: "this zip compression method" }` at read |
| 17 | Names on disk | `LocalVfs::child` on Windows refuses names that are not file names there (`< > : " \| ? *`, control characters, a trailing dot or space, a device name: `CON`, `PRN`, `AUX`, `NUL`, `COM1`…`COM9`, `COM¹`…`COM³`, `LPT1`…`LPT9`, `LPT¹`…`LPT³`, `CONIN$`, `CONOUT$`, in any case, also with an extension): `InvalidName`, an item error when a copy out meets one. The check is a pure function, tested on every platform; `windows_device_name` is the one list, and session names ask it too |
| 18 | Modes on disk | Copied out of an archive to a back-end with Unix modes, a member's mode keeps its `rwx` bits and only those the platform gave the new target: `mode & 0o777 & allowed`, `allowed` being the target's mode as created, with each `r` standing for its `x`, since a file is created without `x`. Never `setuid`, `setgid` or sticky, nor more than the umask gives, also with `--preserve`. The new target carries the umask and any default ACL, so no call and no environment is needed (T1). A target that shows no mode counts as umask 022. The listing still shows what the archive states. Between archives the mode stays as it is |

**Limits** (`ArchiveLimits::default()`, overridable in `archives.toml`, §1.5.12):

| Field | Default | Key | Range | Why |
|---|---|---|---|---|
| `entries_max` | 1,000,000 | `limits.entries_max` | 1,000…100,000,000 | ten times G6; synthesized folders count |
| `stream_max` | 64 GiB | `limits.stream_max_gib` | 1…4,096 | one decoding pass of a large compressed backup |
| `cache_max` | 8 GiB | `limits.cache_max_gib` | 1…1,024 | also the largest member that spools |
| `ratio_max` | 2,000 | `limits.ratio_max` | 0…1,000,000 | deflate ends at 1,032, so only bzip2, xz and zstd extremes trip it; a gzip bomb is stopped by the sizes |
| `ratio_floor` | 64 MiB | none (a test parameter) | | small streams of zeros are normal |
| `nesting_max` | 8 | `limits.nesting_max` | 1…32 | far above real nesting |
| `paths_max` | 256 MiB | `limits.paths_max_mib` | 1…65,536 | 268 bytes of path per entry at `entries_max`, folders included; with the nodes, 512 bytes per entry allowed |
| `depth_max` | 256 | `limits.depth_max` | 16…2,048 | far above real trees; a 4,096-byte name can reach 2,048 |

**New errors** (`VfsError`, `#[non_exhaustive]`; §1.3.2 rules, exit codes unchanged):

| Variant | `kind()` | Message | `io::Error` |
|---|---|---|---|
| `ArchiveCorrupt { spec, reason: String }` | `invalid_data` | `{spec}: not a valid archive: {reason}` | `InvalidData` |
| `ArchiveLimit { spec, limit: &'static str, value: u64 }` | `too_large` | `{spec}: archive limit exceeded ({limit}: {value}); see archives.limits` | `FileTooLarge` |
| `ArchiveEncrypted { spec }` | `unsupported` | `{spec}: the member is encrypted and cannot be read` | `Unsupported` |

**Warnings (D-145).** New `WarningCode`s, appended: `archive_member_refused`, `archive_duplicate_member`, `archive_encrypted`,
`archive_symlink_skipped`, `archive_scan_limit`, `archive_trailing_zeros`. A warning is raised once per index (or operation), kept in the `ArchiveVfs` and
drained by `Vfs::take_warnings`, which the scan calls at its end and the `Service` at the end of a `vfs`, `quick` or `text`
command, so `warnings` in JSON is complete and a listing stays pure.

#### 1.5.8 Time (normative; D-146)

The zone is `(config.clock)().time_zone()` when an index is built (T1); no call reads the system zone. Order for a zip
entry; the first that gives a value wins:

1. NTFS extra field (0x000a): modification and creation time, 100 ns (`mtime`, `ctime`).
2. Extended timestamp (0x5455), central form: modification time in Unix seconds (`mtime`).
3. DOS date and time: **local wall time**, read in the injected zone by `tz.to_ambiguous_timestamp(dt).compatible()` (a
   time in a gap moves forward with the old offset; a time in a fold takes the earlier instant). Resolution 2 s; FC-021's
   default tolerance of 2 s covers it.
4. Otherwise `None`. Invalid DOS fields are no time: month 0 or over 12, day 0 or past the month, hour over 23, minute over 59,
   second field over 29; the all-zero date too.

Tar: a PAX `mtime` record (decimal, with a fraction) that parses, else the header's seconds (UTC, including 0, the epoch).
Any time outside the years 0000–9999, which RFC 3339 can write, is `None`, for every format: an archive can state any
instant, and the `@<secs>` form of `FileTime` must not reach listings or JSON from hostile input. Single streams: gzip `MTIME` when not zero, otherwise the container's `mtime`. A
written zip member (W5b) stores DOS time in the injected zone, clamped to 1980–2107, **plus** a 0x5455 field, so a reader in
another zone gets the same instant; a time outside the field's `u32` range writes no field. Written tar headers hold whole
seconds. Tests use `TimeZone::fixed` and POSIX strings (`TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3")`), so no
time-zone database is needed on any platform.

| # | Stored | Zone | Result |
|---|---|---|---|
| 1 | DOS 2026-07-15 12:00:00 | UTC+2 | `2026-07-15T10:00:00Z` |
| 2 | same | UTC−5 | `2026-07-15T17:00:00Z` |
| 3 | DOS 2026-03-29 02:30:00 (gap) | CET rule | `2026-03-29T01:30:00Z` |
| 4 | DOS 2026-10-25 02:30:00 (fold) | CET rule | `2026-10-25T00:30:00Z` |
| 5 | DOS with 0x5455 = 1,792,415,520 | any | `2026-10-19T13:12:00Z`, zone unused |
| 6 | DOS, 0x5455 and NTFS with 500 ms | any | the NTFS time, with its fraction |
| 7 | DOS month 13, day 0, second field 30, date 0 | any | `None` |
| 8 | malformed 0x5455 (length 3) | UTC+2 | the DOS time |
| 9 | tar header 1,792,415,520 + PAX `mtime=1792415520.5` | any | `2026-10-19T13:12:00.5Z` |
| 10 | tar header 0 | any | `1970-01-01T00:00:00Z` |
| 11 | PAX `mtime=abc` | any | the header's time |
| 12 | gzip `MTIME` 0, container 12:00 | any | the container's time |

#### 1.5.9 CRC (D-147)

| Source | `Entry::crc32` | `CRC_PROVIDED` |
|---|---|---|
| zip | the central directory's value, without reading; `None` for folders and encrypted members | yes |
| tar family, single streams | computed by the index pass (`Entry` documents "computed earlier in this run"), so no second decode | no |
| a member in a cache file | as its archive | as its archive |

`CrcComparator` needs no change: `VfsSource::known_crc32` returns the value and `crc_result` compares it **as it is**. Zip against
zip reads nothing; zip against a local file computes the local side only (FC-025). A corrupt zip member with a correct
central CRC is therefore "same" for the CRC test: that is what the test promises, and the binary test decodes it. `quick
--crc` is the same comparator. `vfs stat` and `vfs ls --json` show the value.

#### 1.5.10 Writes (W5b; D-148, D-149, D-150)

Writes exist for zip and the tar family (Q8, D-63). A primitive on a single-stream archive, or below one →
`OperationUnsupported`; `Dos` attributes other than `R`, `created` times and `move_to_trash` → `OperationUnsupported`,
as the capabilities say. A member write never touches the archive until the rebuild.

```rust
// hikasa_vfs::vfs (additive)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish { Commit, Abort }
pub trait Vfs: Send + Sync {
    /// Opens a batch on the archive `at`: until `finish` the primitives on its members only change an overlay,
    /// which every reader of this back-end sees. Another batch on it → `Unavailable`. Default: nothing.
    fn begin(&self, _at: &Spec) -> Result<(), VfsError> { Ok(()) }
    /// Ends the batch: `Commit` rebuilds `at` (the W3 hook, §1.3.10), `Abort` drops the overlay. Default: nothing.
    fn finish(&self, _at: &Spec, _how: Finish) -> Result<(), VfsError> { Ok(()) }
    /// Creates an empty archive at `spec`, whose name matches a writable mask. Default: `OperationUnsupported`.
    fn create_archive(&self, spec: &Spec) -> Result<(), VfsError> { Err(op_unsupported(spec, VfsOp::Mkdir)) }
}
/// Gains `Seek` (the zip writer patches local headers). `DirectWriter` answers `Unsupported`; `MemWriter` keeps a position.
/// `sync` (default: nothing) flushes and syncs without touching the target, so a look at the target can follow the wait.
pub trait AtomicWrite: Write + Seek + Send { fn sync(&mut self) -> Result<(), VfsError>; fn commit(self: Box<Self>) -> Result<(), VfsError>; }
```

`Vfs` also gains `lock_writer(&self, spec) -> Result<Option<Box<dyn WriteLock>>, VfsError>` (D-157; default `Ok(None)`) and
`shadows_of(&self, spec) -> u64` (the records a member shadows, for `--dry-run`; default 0).

**Batches (D-148).** `begin` and `finish` keep "one rebuild per operation" and stop a write from vanishing: a
primitive or an `open_write` commit **outside** a batch is its own batch and rebuilds at once, so `--output` into an
archive and W10's save need nothing special. `begin` on an archive with a batch of another operation → `Unavailable
{ reason: "another operation is changing this archive" }`. `ops::execute`, the single place an operation ends, runs:

1. Before an item touches a spec with `Spec::container() = c` (target on `dst`, source on `src` for moves and
   removes): the first time, `vfs.begin(c)`. An error fails the item.
2. Items run as in W3 and change only the overlay. **Moves into an archive run in two phases:** the copy parts first; the
   source removals (`move` sources, then `remove_dir`, plan order) only after the target's rebuild succeeded, so a failed
   rebuild leaves every source in place. A removal that fails then fails its item (the copy stays).
3. At the end, roots in this order: every target root, then source-only roots. `finish(root, Commit)`; with
   `cancelled`, `finish(root, Abort)`, and a source-only root is aborted too when a target root failed.
4. `Abort` or a failed `Commit`: the `done` items of that root become `failed` with that error (`cancelled` →
   `not_run`), the summary is recounted, `ok()` is false, exit 100. Nothing is partly written. With an item failed
   and others done, the done ones are committed (per-item results, ARCH-101). A cancel between two target roots: the
   rebuild that went through stands, the sources of its moves stay, and those items are `not_run`: no item says done
   with its source in place.
5. A guard aborts every open batch if `execute` unwinds; a change outside `execute` has a guard of its own.

Nested archives: `begin(c)` on `a.zip/b.tar` opens the chain up to the outermost file. **Writers of other back-ends and
processes (D-157):** a batch takes the writer's lock of its outermost file first and reads the stamp second, so the stamp
holds for the batch. `Unavailable` as above, at once: a writer that waited would find the archive changed since its plan.
A rename that would replace an archive around its own source, or a level of its chain, is refused at planning, whatever
the overwrite policy, also through a link where the file's identity is known: `InvalidTarget "is the archive that holds
the source"`; so is a move of a file into itself (`"is inside the source"`).

**Overlay (`overlay.rs`).** On the first change of a batch the member tree is cloned into a mutable tree whose nodes
are `Kept(index)`, `New { cache file, mode, mtime }`, `Dir`, `Link`. Primitives follow the semantics of `MemoryVfs`
(§1.3.1, D-94): `remove` of a non-empty folder → `NotEmpty`; `rename` moves a subtree; existing targets as W3;
`open_write` spools into a cache file (`reserve` per 1 MiB) and enters the tree on `commit`; a link at the target is
replaced, never followed, whatever `replace_link` says. Implicit folders become explicit only when a primitive
names them. `create_symlink` adds a link member. New member defaults: mode `0o644`, folders `0o755`, the clock's now.
Staged content follows the cache rules; `Abort` releases it.

**Rebuild protocol (normative; D-149).** `finish(at, Commit)`:

1. An overlay with no net change: nothing is written.
2. Re-stat the container. A stamp (length, time, and the file's identity where the back-end knows one) other than the
   batch's → every done item fails `InvalidTarget "changed since the plan was made"`, nothing is written. A damaged archive
   (rule 9) → `ArchiveCorrupt`.
3. `w = base.open_write(container, { overwrite: true, create_parents: false, replace_link: false })`: a temporary sibling
   of the file, the file a link names being the target (Q8, `docs/11` "Local writes"). For a nested archive `w` is a cache
   file.
4. Write the archive (below). Any read, write or cancel error drops `w`: the archive is untouched (INV-VFS-3).
5. `w.sync()`, the stamp of step 2 again, `w.commit()`: `fsync`, one last look, atomic rename. For a nested archive the
   result enters the parent's overlay as a `New` member and step 1 repeats for the parent, also a parent that had no change
   of its own or no overlay yet, until no level below the outermost has a change; only the outermost `commit` touches a real
   file, so a chain is atomic too. A change that does not reach the outermost level is an error, never a success.
6. Drop the index and the cache files of the old stamp. End the writer's lock, which removes its file.

| Format | Kept members | New or changed members | Stream |
|---|---|---|---|
| zip | `raw_copy_file` in central-directory order, no recompression; renamed ones `raw_copy_file_rename`; the archive's comment kept | after the kept ones, sorted by path: `start_file` with `Deflated`, level 6, `large_file` from 4 GiB; a 5-byte 0x5455 field by `add_extra_data`; UTF-8 names; a folder by `add_directory`, a link by `add_symlink`. A touched member (`set_times`, `set_attrs`) is decoded and written again: `raw_copy_file_touch` would keep a stale 0x5455 field | `ZipWriter::new(w)`, seekable |
| tar | the raw blocks of each member (extension headers, header, data, padding) in archive order, by one forward pass; a renamed or touched one: a new header, its data range | after the kept ones, sorted by path: `tar::Builder` with `HeaderMode::Complete`, uid and gid 0 | plain |
| tar.gz | same | same | `GzBuilder` with `mtime(0)` and no name, level 6 |
| tar.bz2 | same | same | `BzEncoder`, level 9 |
| tar.xz | same | same | `XzWriter`, `XzOptions::with_preset(6)`, CRC64 |
| tar.zst | same | same | `zstd` level 3, checksum on |

- Levels are the formats' usual defaults, constants of `rebuild.rs`, not options. The same inputs give the same bytes (given
  the same crate versions).
- **A rebuild keeps what it did not touch:** refused, encrypted, sparse and link members go on as raw data; a shadowed
  duplicate shares its winner's fate (D-156): the warning `<n> shadowed members went with the member that replaced them`
  names only what really left this archive, whether it ended or travelled to another one; `--dry-run` says beforehand which
  (`would go`, `would move to the other archive`), and `<n> shadowed members of <source> cannot go to <target> and end with
  this move` names what a plain file or a record without content cannot take. Records keep their order, a rewritten one its place
  in the pass, and new members come last, so the last record still wins. A replaced member takes its shadows along, so what
  was overwritten does not live on in an older record. CRCs of new members come from the writers; kept zip members keep
  theirs.
- **A renamed or touched tar member keeps what still holds.** With its path unchanged its name blocks (a GNU long name, a
  PAX `path`) and the name and prefix of its header stay as bytes, so a name that does not decode survives a touch. With a
  new path the fields are cleared first: a ustar `prefix` left behind would join the new name and write the member to
  another path. A move onto text made from a name that does not decode (U+FFFD in `to`) is refused.
- **A refused member stays, whatever goes.** One that was refused only for a path conflict reads as a member once the
  path is free: the operation that freed it warns (`archive_member_refused`). A name refused by rule 1 stays refused.
- A zip with a prefix (`ZipArchive::offset() != 0`, a self-extractor) → `Unsupported { what: "rewriting a zip with a
  prefix" }`, nothing written. Signed `.jar`, `.apk` lose their signature's validity when rebuilt (`docs/11` says so).
- The fresh archive is not re-read before the rename: every path through the writers is covered by round-trip
  properties, and verifying would double the disk use.
- A rebuild's cost is the size of the archive, not of the change: `Phase::Writing` reports the archive's bytes after
  the items. Cancellation is checked per 1 MiB and before `commit`; a cancelled rebuild is `Abort`.

**Empty archives (FC-077; D-150, revises the hook of §1.3.10).** `create_dir` stays literal: `ops` also calls it for a real
folder named `x.zip` inside a copied tree, which must stay a folder. **`plan_mkdir` decides:** a target whose final name
has `vfs.archive_format(name)` with `writable()` gets the new `Action::CreateArchive` (`create_archive`, human word
`mkarchive`, `kind: file`), run through `Vfs::create_archive`, which writes an empty archive through `open_write`. An
existing item: the W3 rules of `mkdir`. Single-stream masks (`x.gz`) stay plain folders. Only the final name counts: a
missing ancestor is a folder, as in `mkdir -p new.zip/sub`. Inside an archive the same rule makes a nested empty
archive. An argument with a trailing separator wants a folder (§1.5.12): `vfs mkdir new.zip/` makes one, inside an archive
too.

| Format | Bytes of an empty archive |
|---|---|
| zip | the 22-byte end record (`ZipWriter::new(w).finish()`) |
| tar | two zero blocks (1,024 bytes) |
| tar.gz, tar.bz2, tar.xz, tar.zst | the same two blocks through the encoder |

**Backups** belong to W6 (§1.5.17): W5 rebuilds without one.

#### 1.5.11 Folder scan (FC-001, FC-015; D-151)

```rust
// hikasa_folder::rules (additive; a key of [handling] and a key of the rules root, both sparse, schema version unchanged)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ArchiveHandling { AsFiles, #[default] OnceOpened, Always }
pub struct Handling { /* … */ pub archives: ArchiveHandling }
pub struct FolderRules { /* … */ pub open_archives: Vec<String> }     // paths relative to the bases, `/`-separated
pub struct FolderRulesPatch { /* … */ pub archives: Option<ArchiveHandling>, pub open_archives: Option<Vec<String>> }
// hikasa_folder::model
NodeFlags::ARCHIVE = 512                                               // name "archive"; FLAG_NAMES grows to 10
```

**Base sides (FC-001).** A base is an `ArchiveMember` root or a folder member (`Service::folder_prepare` resolves with
`SpecUse::Folder`, so `folder compare a.zip b/` works whatever the handling mode). `load_base`, capabilities, case
resolution (`CaseMode::Auto`: sensitive only when both sides are), `effective_criteria` and the content engines work
unchanged: a base that cannot be read → `BaseNotLoadable`, exit 107.

**Modes (FC-015).** Headless default `OnceOpened` with nothing opened gives M1's results: an archive inside a tree is a file
(goldens unchanged).

| Mode (`--archives`) | Archive in a tree |
|---|---|
| `AsFiles` (`files`) | a file, always: size, time, content tests of the file |
| `OnceOpened` (`opened`, default) | a file, until its path is in `open_archives`, then a folder |
| `Always` (`folders`) | a folder |

**Algorithm** (one place in `scan.rs`, after a side is listed, so the flat mode gets it too):

1. For each `File` entry: `vfs.archive_format(name)`; `None` → nothing. The rest uses the entry's fact `archive`:
   `NodeFlags::ARCHIVE` on the node, whichever side has it.
2. It **opens** when the mode says so (its relative path on either side, as written in `open_archives`) and the budget of
   step 5 allows. Then `entry.kind = Dir`, `entry.size = 0`, and `spec_of` gives `Spec::archive_member(child, "")`.
3. The prefilter (D-45) passes such a name with kind `Dir`, so the folder masks decide, as for a folder. An archive
   that stays a file meets the file masks.
4. Alignment, FC-054, counters and states see a folder pair. A listing that fails (corrupt, encrypted index, limit)
   takes the existing path: the node's `ScanStatus::Failed`, the side's error flag, a `NodeError`, an **error row**; the
   scan goes on. Rows `<refused-N>` and `<damaged>` of `Listing::errors` arrive as entry errors.
5. **Budget.** Entries inside archives add to one atomic counter per scan. Before an archive opens, its index is built
   (`Vfs::archive_entries`, so this happens on the folder's task) and its whole count, synthesized folders included, is
   reserved; counting at the listing would come after every archive of a folder had opened. An archive that does not fit in
   `SCAN_ARCHIVE_ENTRIES_MAX` = 5,000,000 stays a file and one `archive_scan_limit` warning is added; a later, smaller one
   may still fit. The archives of the bases open whatever the budget says and count first. An archive that cannot be
   indexed counts 0: its listing fails, which is its error row. With `nesting_max` and the cache limits this bounds a
   recursive archive in `Always` mode.
6. At the end `vfs.take_warnings()` joins the comparison's warnings.

Nested archives follow the same rule inside archives: `a.zip/b.tar.gz` opens in `Always`, or through the entry
`b.tar.gz` below an opened `a.zip`, written `a.zip/b.tar.gz` in `open_archives`. Links inside archives stay `Symlink`
nodes compared by target text, also with follow-symlinks on. FC-014 flatten descends opened archives like folders, and
their members are in the Path column. FC-025: §1.5.9. The memory budget of content tests (D-25) applies as for files.

#### 1.5.12 Service and settings

`archives.toml` gains a section, schema version unchanged (an additive known section, as D-108):

```toml
schema_version = 1
[types]                  # unchanged
[limits]                 # OPT-010; §1.5.7
entries_max = 1000000
stream_max_gib = 64
cache_max_gib = 8
ratio_max = 2000
nesting_max = 8
paths_max_mib = 256
depth_max = 256
```

`ArchiveLimitsSection` in `settings/archives.rs` with the ranges of §1.5.7 (`validate`, one `KindRule` for `limits.*`, OPT-010,
P0); `ArchiveTypes::masks(&self) -> ArchiveMasks`, `ArchiveTypes::limits(&self) -> ArchiveLimits`; `ArchiveType` maps one to one
to `ArchiveFormat`. The Options page `archive_types` keeps its table of masks (no new dialog); limits are `settings` keys. A
package (D-86) carries the file unchanged. D-86 itself is unchanged: its import still accepts only stored and deflate
entries, although the workspace `zip` now has more codecs (test).

```rust
impl Service {
    pub fn parse_spec(&self, input: &OsStr) -> Result<Spec, ApiError>;              // + resolve_spec: Any, or Folder
                                                                                   //   when the input ends in a separator
    pub fn parse_spec_as(&self, input: &OsStr, use_: SpecUse) -> Result<Spec, ApiError>;
    pub fn shutdown(&self);                                                         // `ArchiveVfs::cleanup`
    pub fn folder_prepare_in(&self, req: &FolderCompareRequest, job: &JobControl)   // `folder_prepare` inside a job
        -> Result<PreparedFolderCompare, ApiError>;
    pub fn vfs_stat_in(&self, spec: &OsStr, follow: FollowLinks, job: &JobControl)  // `vfs_stat` inside a job
        -> Result<VfsStatDto, ApiError>;
}
```

- `Service::build` reads `archives.toml` (a broken file: defaults and one `invalid_config_value` startup warning, a comparison
  of plain files must not depend on it), builds `ArchiveConfig` with `Clock` and `config.cache_dir.join("archives")`, and
  `VfsRouter::with_archives`. `rebuild_router` keeps the `Arc`. `reload_settings` calls `set_config`.
- `SpecUse::Folder`: `folder_prepare`, `vfs ls`, a `cp`/`mv` target with a trailing separator, the specs of a Folder Compare
  view. Everything else `Any`.
- Every place that uses `Spec::parse_os` directly on a user spec resolves too: the targets of W3's write commands (they
  are still parsed without reading stdin) and `input_stamp` (`api/settings.rs`).
- `infer_view_kind` (D-50) gains: two files that are both archives by name, or one archive and one folder → `FolderCompare`.
  `--fv` still wins. `gui open` sends the resolved canonical spec.
- Sessions store specs as canonical text (D-80), so `a.zip/inner/x` and `a.zip/` round-trip (§1.5.2).

#### 1.5.13 CLI and JSON (D-152)

No new subcommand and **no new schema**: `SCHEMAS` stays at 35; the shapes do not change, `spec` fields show archive
specs. Exit codes are the table of M1 §2.21: every `vfs` error or failed item 100; `quick`, `text compare` loading errors 100; a
`folder compare` base that cannot be loaded 107; error rows as in M1.

| Command | Behaviour in W5 |
|---|---|
| `hikasa vfs ls a.zip`, `a.zip/`, `a.zip/inner` | lists members, `SpecUse::Folder`; `<refused-N>` and `<damaged>` in the listing's errors, as M1's entry errors |
| `hikasa vfs stat a.zip` | the file; `a.zip/` the root, with the format's capabilities; `a.zip/x` the member, `crc32` included |
| `hikasa vfs cat a.zip/x`, `x.txt.gz/x.txt` | the member's bytes; `Symlink`, `Other`, encrypted: error, exit 100 |
| `hikasa folder compare a.zip b/`, `a.zip/inner b/`, `a.zip b.tar.gz` | §1.5.11; new `--archives files\|opened\|folders`, repeatable `--open-archive <PATH>` |
| `hikasa quick a.zip/x b/x`, `hikasa text compare a.zip/x b.tar.gz/y` | as files; format detection uses the member's name |
| `hikasa quick a.zip b.zip` | the two files: `SpecUse::Any`, by design |
| `hikasa vfs cp out.txt a.zip/dir/` (W5b) | D-102 naming over the resolved target; `-p` for a missing `dir/`; `hikasa.vfs_op.v1`; one rebuild after the items |
| `hikasa vfs cp a.zip/x out/`, `cp -r a.zip/ out/` | W5a: reads only; link members skipped (rule 5) |
| `hikasa vfs mv`, `rm`, `touch`, `mkdir` on members (W5b) | as W3, through batches; `--trash` on a member: item error `unsupported` |
| `hikasa vfs mkdir new.zip`, `new.tar.gz` (W5b) | item `create_archive`, human `mkarchive new.zip`; summary `created` |

`--archives` and `--open-archive` are in `folder compare` and `session new|edit` rule keys (`handling.archives`, `open_archives`);
the word list is parsed by core (`vocab::parse_archive_handling`, T11; architecture check 7): `--open-archive` with `files`
adds `switch_ignored`. A dry run prints the plan (T12); a rebuild is not an item, and only a failed one changes results (the items
take its `error`, the shape of W3). `action` gains `create_archive`, an additive value of `hikasa.vfs_op.v1`.

**Text in JSON.** Names reach `--json` from hostile archives. `write_json` escapes, beside what JSON requires, DEL, the C1
controls, U+2028, U+2029 and the bidirectional controls (U+061C, U+200E, U+200F, U+202A…U+202E, U+2066…U+2069) as `\uXXXX`.
A parser gets the same string back, so no consumer loses anything, and a terminal or an editor showing the document acts
on none of them. Decision for bidi: escaped like the controls, because `txt.exe` shown as `exe.txt` is the same attack as
a CSI, and the escape is lossless.

Human output adds nothing except the `mkarchive` word. Warnings go to stderr as usual. **Catalogue:** no entry names
archives and no status changes: `NewFolder` stays `planned` until W9, `Open` and `Expand` are `implemented` already, so
`docs/reference/commands.md` does not change. **Help:** new topic `archives` (formats, masks, specs, nesting, modes, limits,
safety, the cache, rebuilds), linked from `index`, `vfs`, `folder-compare` and `settings`; `help/vfs.md` loses the "not supported"
sentence on archive members; `cargo xtask help-check` stays clean.

#### 1.5.14 GUI (D-153)

Only what the archive sides need, in the existing views; no new dialog, command, page or setting, and no logic (T11):

- **Folder view.** A row whose node has `NodeFlags::ARCHIVE` shows the new in-house zipper icon (D-114, on the 16 × 16 grid,
  in the row's icon place; light and dark). Error rows, `<refused-N>` and `<damaged>` use the existing error row.
- **Opening.** With `OnceOpened`, `Open` (double-click, Enter) on a closed archive row, the right place for FC-015's "expand on
  double-click", adds the row's path to the view's `rules.open_archives` and runs the comparison again by the view's reload
  path; the new folder row is shown expanded. In `AsFiles` `Open` is as before. W9 replaces the full run by a subtree refresh
  (W6). Which rows offer it is `FolderRow::opens_archive`, computed from the flag and the rules in the view model.
- **Text view.** Members load through the VFS; the title shows the canonical spec. The view is read-only as all views are
  before W11. Watching inputs (D-118) uses `stat` of the member, which re-stats the archive: no content is read.
- **Sides.** Path fields of Home and the management panel, command line, IPC and drops pass through `parse_spec` and
  `infer_view_kind`. The base label of a Folder Compare shows the canonical spec.
- **Exit.** `Service::shutdown` after the last window closes.

All of this is view-model tested without a window (§6.5 A5.19).

#### 1.5.15 Dependencies and architecture test (D-154)

Versions checked with `cargo info` on 2026-10-31. Added to `[workspace.dependencies]` with fixed features (architecture
check 3 refuses member-level features):

| Crate | Version | Licence | Features | Why these |
|---|---|---|---|---|
| `zip` | 9.2.0 (from 9.1.3) | MIT | `default-features = false`, `deflate`, `deflate64`, `bzip2`, `zstd`, `lzma`, `xz`. No `aes-crypto`, `time`, `ppmd`, `rayon`, `legacy-zip` | reading what zip tools write; encryption is reported, not implemented |
| `tar` | 0.4.46 | MIT OR Apache-2.0 | `default-features = false` (no `xattr`) | `Header`, `PaxExtensions`, `Builder` |
| `flate2` | 1.1.10 | MIT OR Apache-2.0 | `default-features = false`, `zlib-rs` | the pure-Rust back-end `zip` already enables; features unify |
| `bzip2` | 0.6.1 | MIT OR Apache-2.0 | default (`libbz2-rs-sys`, a Rust port; no C) | `bzip2/default` is what `zip` uses |
| `lzma-rust2` | **0.23.0** | Apache-2.0 | `default-features = false`, `std`, `encoder`, `optimization`, `xz` | not the newest 0.24.0: `zip` 9.2 needs `^0.23`, and one LZMA codec in the tree is the point (plan §3). Moves when `zip` moves. Pure Rust (D-63) |
| `zstd` | **0.14.3** | BSD-3-Clause | `default-features = false` | not the 0.13 of plan §3: `zip` 9.2 needs `^0.14.2`, and the licence is not the plan's MIT. `zstd-safe` 8.2.0, `zstd-sys` 2.1.2 (BSD-3-Clause), libzstd 1.5.7 |

`hikasa-vfs` manifest += `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `zstd`, `jiff` (`ArchiveConfig::clock`, the zone). New crates
in `Cargo.lock` besides these: `deflate64`, `libbz2-rs-sys`, `sha2`, `digest`, `block-buffer`, `hybrid-array`, `typenum`,
`const-oid`, `crypto-common`, `zstd-safe`, `zstd-sys`, and the build-time `cc`.

**Licence: one exception, for the maintainer.** `libbz2-rs-sys` is under `bzip2-1.0.6`, a permissive BSD-style licence that
`deny.toml` does not list (verified: `cargo deny check licenses` rejects it, and plan §3 only named the wrapper's
licence). No bzip2 back-end avoids it, the C original has the same licence. The contract adds a crate-scoped
`[[licenses.exceptions]] crate = "libbz2-rs-sys", allow = ["bzip2-1.0.6"]`, as for `epaint_default_fonts`. W3's "stop and
ask" does not block: this row is the question, and it stands unless the maintainer objects. `packaging/third-party-licenses.py`
needs no change (the crate ships `LICENSE`). `cargo deny check` passes with no other new exception.

**`zstd` bundles C.** `zstd-sys` compiles libzstd with the `cc` crate at build time: no cmake, no bindgen (pregenerated
bindings), no nasm. What each build needs is a C compiler for the target. Verified on 2026-10-31 with this feature set in a
scratch crate, each run printing `1.5.7`:

| Target | How packaging or CI builds | Result | Needs |
|---|---|---|---|
| `x86_64-unknown-linux-gnu` | C by the host `cc`, link by `zig cc` (`packaging/zig-cc.sh`, glibc 2.31) | builds, glibc floor 2.30, no BMI code | host `cc`. `packaging/build-linux.sh` gains `export CFLAGS_x86_64_unknown_linux_gnu="-march=x86-64 -mtune=generic"`, because a machine-wide `CFLAGS` with `-march=native` would reach the C objects, which `RUSTFLAGS` does not govern (`check-linux-binaries.sh` still checks) |
| `x86_64-pc-windows-gnu`, `i686-pc-windows-gnu` | mingw gcc, `+crt-static` (`packaging/build-windows.sh`) | build, run under wine, system DLLs only | the mingw gcc the scripts already require |
| `x86_64-pc-windows-msvc` | `cargo xwin` here; `cl.exe` on `windows-latest` | builds | the MSVC tools `link.exe` already needs |
| `aarch64-apple-darwin` | `zig cc` here; Xcode's `clang` on `macos-latest` | builds and links | the Command Line Tools rustc already needs for linking |

zstd's one assembly file is used on x86-64 outside Windows only and goes through the same C compiler. Do not set `CC` to
`zig cc` for Linux: the `cc` crate passes `--target=x86_64-unknown-linux-gnu`, which zig refuses. Plan §5.1's
fallback `ruzstd` is not needed. LZMA stays free of C (D-63); libzstd is the only C code in Hikasa's binaries, and no Hikasa
source is `unsafe` for it.

**Architecture test.** Manifest table as above. Check 2 gains a confinement rule: the tokens `zip::`, `tar::`, `flate2::`,
`bzip2::`, `lzma_rust2::`, `zstd::` only under `core/hikasa-vfs/src/archive/` (`zip::` also in
`core/hikasa-session/src/package/`, D-86). `docs/02`: the lines of the table, the `zip` row updated, the C note, the
exception.

#### 1.5.16 Gate G6 and budgets (D-155)

Measured at the end of W5a on an idle machine, release build, fixtures generated by the test; recorded in `docs/02`;
`bench.yml` carries them: the 25 % tolerance of D-64 is criterion's noise threshold and covers the benches only; the gates of
`perf_gates.rs` compare with their absolute thresholds.

| Item | Measure | Where | Threshold |
|---|---|---|---|
| G6 list | `hikasa vfs ls big.zip --json` on a zip of 100,000 stored files in one root, wall time of the process, warm cache | `cli/hikasa-cli/tests/perf_gates.rs`, `#[ignore]` | < 1 s |
| G6 compare | `hikasa folder compare tree/ tree.zip --content crc --json`, 10,000 files of 4 KiB in 100 folders, equal sides | same | < 2 s |
| G6 CRC | `ArchiveVfs::prepare` of a 1 GiB `tar.gz` of one member (repeating pseudo-random text, ratio ≈ 4:1, the CRC in the index) against decoding the same stream to a sink with `MultiGzDecoder` | `core/hikasa-vfs/benches/archive_crc_1gb.rs` | ≤ 1.3 × the raw decode |
| G2 | the M1 gate again: every file now meets `archive_format` | `perf_gates.rs` | unchanged |
| Memory | an index as §1.5.5 bounds it; peak RSS of G6 list ≤ 150 MiB; of G6 CRC ≤ 64 MiB (nothing spooled) | `perf_gates.rs`, RSS from the existing helper | listed |
| Cancel | `prepare` and a spool of a 1 GiB stream stop within 100 ms | `archive_cache.rs`, name holds `cancel` (D-69) | G4 |

Measured in the scratch crate: `ZipArchive::new` 14 ms and `by_index_data` of all entries 1 ms for 100,000 entries, so the gate has
room for the tree and the JSON. Rebuilds have no gate: their cost is one copy of the archive (`docs/11`).

#### 1.5.17 Hooks for W6

- **Operations inside archives.** W6's `OpPlan` wraps `ops::Plan`, so its items reach `execute` and get batches, the two-phase
  move and item results for nothing. `Capabilities` per spec (`REMOVE`, `RENAME`, `WRITE`) grey out what a format lacks,
  so single-stream archives are read-only with no special case.
- **Backups.** W6 inserts one step in `execute` just before `finish(root, Commit)`: one backup of `root.outermost_file()` per
  operation, when the backup option asks. A backup of the archive replaces per-member backups inside it; W6 decides
  whether a `replaces` item inside an archive also keeps a copy.
- **Recycle bin.** `move_to_trash` stays unsupported on members; FS-005's "delete to recycle bin if possible" is W6's
  confirmation choice (§1.3.10), a member being deleted for good only after a rebuild.
- **Open With, external conversions.** They need a real path. W6 adds `ArchiveVfs::extract_to_cache(spec, cancel) ->
  Result<PathBuf, VfsError>` on the existing spool; the path is valid for the session.
- **Refresh.** `Open` on an archive row re-runs the comparison (§1.5.14); W6's subtree refresh replaces it. An archive
  rebuilt by an operation changes its stamp, so the refreshed subtree reads the new index.
- **Sync.** FS's *Sync within archives* is a mode of W6's plan over `SpecUse::Folder` sides; D-151 already compares them.

#### 1.5.18 Documentation updates of W5

`docs/01` §3.1 (`ArchiveMember { archive, member }`, root `""`), §3.2 (the new trait methods, `AtomicWrite: Seek`); `docs/02` (§1.5.15, the
G6 measurements); `docs/03` §5 (`archives.toml` `[limits]`, the cache folder); `docs/04` FC-001, FC-015 (modes, opening,
`open_archives`, the budget), FC-025 (the zip CRC as it is), FC-077; `docs/05` TC-002; `docs/10` (format detection by the member's
name); `docs/11` §1 (the Archives row and VFS-002: formats, capabilities, cache, safety and limits in user terms, rebuild cost, a
signature does not survive; "Mutations": batches), §2 (how `…/a.zip/inner/x` resolves, the trailing separator, nesting,
case); `docs/12` §1 and §4 (`vfs` rows, `--archives`, `--open-archive`, `create_archive`, `mkarchive`); `docs/14` (the zipper icon,
`Open` on an archive row); `docs/15` OPT-010 (`[limits]`); `docs/16` M2 progress; `help/` as §1.5.13; `CHANGELOG.md` per
part. `docs/reference/commands.md` is unchanged.

#### 1.5.19 W5 notes (implementation)

**W5a.** Where the implementation interpreted or extended the text above:

1. **`SharedReader` holds no file between calls.** It is `Arc<Mutex<Option<Box<dyn ReadSeek>>>>` plus a position, and a
   `ReaderLease` opens it for one call: an index kept for a session must not keep the container open (Windows: the
   file could not be removed or replaced). Not the `Arc<Mutex<Box<…>>>` of §1.5.5.
2. **PAX records are parsed in-house** (`tar_walk::pax_records`), not by `tar::PaxExtensions`, which splits on newlines,
   so a `path` holding one would fall back to the truncated header name and two readers could disagree. `tar::Header`
   is still the only use of the crate when reading.
3. **Rule 8 against rule 9.** A stream or tar that fails at offset 0 with no member is `ArchiveCorrupt` (rule 8), also
   when it is a real gzip cut inside its header: the two cannot be told apart. A damaged **single stream** has no
   complete member, so its tree is empty and the root shows `<damaged>`. A5.10's cuts accept either answer below the first
   decoded byte.
4. **Before `ZipArchive::new`** the end records in the tail of the file are read (`stated_entries`): rule 13 says memory
   stops at `entries_max`, and the crate would read a central directory as large as the file.
5. **Zip fields read by `central_times`** also include the flags, the method and the DOS date and time: the crate's
   `DateTime` loses invalid fields' meaning, the method tells AES (99) without an AES extra field, and the UTF-8 flag
   decides the text of rule 1.
6. **Rule 15 covers streams.** LZMA and xz **zip members** go through the crate's decoder, whose dictionary grows with
   the output through `try_reserve`, so memory is bounded by the guards and a failed reservation is an error; zstd
   members keep libzstd's default limit of 128 MiB.
7. **Raw names stay in the container**, not in the tree: a rebuild copies raw records, and `Tree::records` lists every
   record of the archive in order with its state (live, refused, shadowed), the seam of W5b. New reason `backslash`.
8. **Internal node bits.** `NodeFlags::LEFT_ARCHIVE_ROOT`, `RIGHT_ARCHIVE_ROOT` (1024, 2048; not in `FLAG_NAMES`) say which
   side of a folder pair is an opened archive: `TreeRead::spec` needs it to build a member's spec, and an archive against
   a real folder of the same name has one on one side only. `spec` now serves archive bases and flatten folders.
9. **`FolderEnv::archive_entries_max`** injects the budget of step 5; the session passes `SCAN_ARCHIVE_ENTRIES_MAX`.
10. **Resolution stops at `..`.** A path that ends in `..` has no parent by its text (D-06; §1.6.25 note 32), so step 3 returns
    the spec as soon as the name in front of `rest` is `..`; every path through step 4 would have given that answer. New
    `LocalPath::parent_in`, `file_name_in` (explicit style).
11. **Jobs.** `Service::parse_spec_in` passes the job's token in `vfs ls`, `vfs stat`, `vfs cat`, `vfs cp`, `vfs mv`, `quick`,
    `text compare` and `folder_prepare_in`, as §1.5.4 asks; those commands then `prepare` before they `stat` (§1.5.5).
    Not cancellable, and bounded by the limits: `Service::parse_spec` and `folder_prepare` without a job, and the target of a
    write command. `Service::take_warnings` drains the back-end; the CLI's `Ctx` adds them to every envelope and to stderr.
12. **`stored_spec`** resolves too (without reading stdin or the clipboard), so `a.zip/` is stored as the root.
13. **`./` gives the root its attributes** only; §1.5.5 says the root's time is the container's.
14. **0x5455 reads as `u32`**, the range §1.5.8 gives the written field.
15. **Unknown tar type flags** are `Other`; `X` is read as `x`; PAX `GNU.sparse.*` records make a member sparse, with the
    name and the size they state, so a PAX sparse member is not read as its packed map.
16. **The index budget** of §1.5.5 counts the tree (`ArchiveVfs::index_memory`). The crate's own metadata of a zip comes on
    top, and what it reads of a hostile directory stops at `paths_max` + 128 bytes × `entries_max`. The RSS budget of G6
    measures a sound archive and bounds nothing on a hostile one.
17. **Additive API** beyond the text: `ArchiveVfs::{counters, cache_used, cache_dir, index_memory, inject_decoder_panic}`,
    `ArchiveCounters`, `VfsRouter::archives`, `hikasa_vfs::archive::{name, time, zip_read}` (public for the tests:
    `windows_name_problem`, `central_times`). Tests: `tests/archive_memory.rs` holds the counting allocator of A5.8.
18. **Open** counts a closed archive row as `ctx.pair_selected`, so an orphan archive can be opened too.

19. **`write_json` streams** (a pass to a sink, then to the output): the 39 MB document of G6 list held in memory put the
    peak RSS at 159 MiB, over the budget of 150.
20. **The fixture of G6 CRC** compresses 3.3:1 at gzip level 6 (four letters at random), the closest a plain text alphabet
    gets to the ≈ 4:1 of §1.5.16.
21. **Green is claimed per part.** W5a's commits are grouped by crate, and A5.20 holds at the end of the part: an
    intermediate commit of W5a (`8beee3d`…`0b37a59`) was not checked on its own, and no note says it builds or passes.
22. **Node text is shared, not split.** The review asked for a parent and a segment per folder. A folder's path is a prefix
    of a member's path, so a node holds an `Arc<str>` and a length: the same cost, and `find`, `children` and the sorted
    key stay as they were.
23. **Sorts stop.** `build_tree` sorts through `sort_checked` (runs of 4,096 with `sort_by`, then merges of indexes), so
    the two sorts of 1,000,000 paths look at `cancel`. Below a run it is `sort_by`.
24. **Cache identity** is a stamp in the file and a reopen, not a handle kept open: one handle per cached file would run
    into the descriptor limit (256 by default on macOS), since a folder comparison can spool thousands of members.

*Seams for W5b.* `ArchiveVfs::on_base` is the one place a primitive or `open_write` on a member is refused; capabilities
come from `ArchiveFormat::capabilities`; `Tree::records` and `DataRef` give each record's range or zip index; `check_item`
already refuses roots; `plan_copy` keeps links between archives; `Cache::writer` can stage files.

**W5b.** Where the implementation interpreted or extended §1.5.10, and why. Item 1 is a deviation from D-149's table; the
maintainer may want to look at it first.

1. **Kept zip members are copied by `zip_write.rs`, not by `raw_copy_file`.** The crate's call builds new headers from what it
   parsed (`ZipFile::options()`), and that is not a raw copy: it drops the encryption flag of a ZipCrypto member, so the
   member reads as garbage afterwards; it keeps only the extra fields the crate knows; it decodes a member's comment; and
   `insert_file_data` refuses two records of one raw name, which refused members may have. The crate's reader shows such
   twins once as well, so a rebuild walks the central directory itself (`read_central`) and matches headers to indexes by
   offset. A record is its local header, data and everything up to the next record (a data descriptor in any of its forms),
   and its central header with only the offset changed; a zip64 field takes the offset past 4 GiB. A header that is not
   there is `ArchiveCorrupt`, nothing written.
2. **A renamed or touched zip member is edited in its headers**, in archive order, and never decoded, instead of
   `raw_copy_file_rename` and "decoded and written again". Name, flags, DOS time, external attributes, a 0x5455 field and
   the time in an NTFS field change; a 0x7075 field, which repeats the old name, goes. So an encrypted member or a method
   this build cannot decode can be moved and touched too. An encrypted member keeps its DOS time, which may be its
   password check; its 0x5455 field gives the new time.
3. **New zip members** go through `ZipWriter` as §1.5.10 says, but into a zip of their own in the cache, whose records
   `ZipOut` then carries over, because `ZipWriter` cannot take records it did not write. The cost is new members written
   twice, and that second copy may pass `cache_max` by their size; the output needs no `Seek`. `AtomicWrite: Seek` is there all the same and `create_archive` uses it. `add_extra_data`
   is deprecated in `zip` 9.2: `add_extra_field`. `unix_permissions` keeps `rwx`, and the crate's reader clears the rest, so
   a zip member's mode stays within `0o777`; in a tar the whole mode stays.
4. **`Vfs::finish_with(at, how, ctl)`** is new beside `finish`: §1.5.10 wants cancellation and `Phase::Writing` from a rebuild, and
   `finish` has no `Ctl`. `ops::execute` calls it; its default calls `finish`.
5. **A batch belongs to the outermost file.** `begin` on any archive of a chain opens it, a level (an overlay) is made on
   its first change, and `ops` opens one root per outermost file, so `a.zip` and `a.zip/b.tar` in one operation are one
   batch and one rebuild, not two that could disagree. `finish` on any archive of the chain ends it all.
6. **A batch has an owner, the thread that opened it.** §1.5.10 says a plain `open_write` cannot be lost, but a write that
   lands in another operation's overlay is lost when that operation aborts. So a change from another thread while a
   batch is open is `Unavailable`, as a second `begin` is. `ops::execute` runs on one thread; W6 must keep it so, or pass
   the batch along.
7. **"Changed since the plan was made" is checked three times**: `begin` compares the stamp with the last one this back-end
   saw, which is the plan's; a commit checks the batch's stamp before it writes and again after the sync, just before the
   rename. The first is an early answer: a long-lived back-end whose other reads moved that stamp on no longer sees a change
   before `begin`, and the per-item re-check of D-103 covers the items. The stamps alone do not protect the data: two
   processes can each pass the last look and then rename, and they did, in 24 of 24 runs (the review's fsync window). What
   protects it from another Hikasa is the writer's lock (D-157), held from before the batch's stamp is read, and a
   change that opens a batch of its own drops any index older than that stamp. The stamps remain the guard against
   writers that do not take the lock; between the last look and the rename there stays a window of one system call that
   no check can close, and a file system without locks has only the stamps.
8. **Shadowed duplicates stay** (D-156). The first version dropped them with a warning, which A5.16 and rule 3 then said; the
   maintainer decided otherwise. `Tree::shadows` has the numbered ones (another raw name for one path, `./a` and `a`) and
   `Tree::hidden` the ones the crate gives no number (the same raw name: it keeps one per name), found by reading the central
   directory once more when it states more records than the crate kept, so an archive without duplicates pays nothing;
   both name their winner. A zip's record numbers are the places in the central directory of the entries the crate kept,
   not the crate's numbers, which stay in `DataRef::Zip`: the crate holds a name at the place of its first record and the
   value of its last, so with spellings that take turns (`a`, `./a`, `a`) only the directory's order says which came
   last. A move to another archive is a copy and a removal (`Vfs::carry_older`, `archive/older.rs`): a zip's records are
   copied raw to a small zip under numbered names and indexed there, which reaches the hidden ones, a tar's blocks are cut
   out of the stream and read back, and the decoded records hang on the new member's overlay item
   (`Item::older`), so a later change of that item in the batch keeps or drops them with it. A tar shadow that moves uses its own record's size: its winner's would be wrong for an older
   version. A hard-linked shadow whose target goes has no member to become, since a file in its place would win, so it goes
   and is counted. `check_counts` and the zip's own count bind kept shadows and hidden records, since the next index reads
   them. A record of a refused name is kept however many have that name.
9. **Hard links** were not in §1.5.10: a kept `1` record names another record by path, so dropping, moving or replacing that
   member would leave a link rule 6 refuses, and the data with it. A link stays a link while its target stays, under its
   name, and is otherwise staged and written as a regular member **at its own place**, from its old header, so owner, group,
   names and PAX records stay, and a link that named it stays one. A moved or touched link takes the same way: its data
   range is its target's, which the forward pass has already left.
10. **Tar gaps.** Global headers and volume labels are no longer part of the next member's blocks (`Pending::first`), and the
    pass copies everything no dropped record owns, so removing the first member of a `git archive` tarball keeps its
    `pax_global_header`. Bytes other than zeros after the end of a tar or a zip are kept too (`Tree::tail`, `records_end`).
11. **A rewritten tar member clones its old header** (owner, names, type, device numbers, link name), and its other PAX
    records and long link stay; with a new path `path` goes, with a new time `mtime`. It is written at its place in the
    pass, with the data as it is. With its path unchanged the name is not rewritten at all (§1.5.10), which also keeps a
    `./` spelling. **A sparse member cannot be rewritten:** its map lies in or after its header, so a rename or a touch
    is `OperationUnsupported`, as is a rename of a folder that holds one, of a member with a sparse shadow, or of a folder
    that holds a member whose name does not decode (a new raw name would have to be made up); so is a touch of an implicit
    folder whose name does not decode, which has no record to edit. All fail at the primitive, before anything changes.
12. **A prefix the offsets count in.** `ZipArchive::offset` is 0 when a self-extractor's writer adjusted the offsets, and the
    stub would go silently, so anything before the first record is refused the same way.
13. **Limits on output** (rule 11–13 of what is written), checked before the commit: the entries and text of the overlay
    plus the refused members' (rows stop at 1,000, past which a hostile archive passed the limit anyway), `stream_max` of
    a new tar stream, `ratio_max` of the result, `depth_max` and 4,096 bytes per name at the primitive, `cache_max` for
    staged content. A zip's records are not decoded output, so a sound zip of any size can be rewritten: its limit
    is its own length plus what is new plus `stream_max`, which only overlapping records can reach.
14. **A nested archive that changes, and then moves or is read whole, in one batch** is rebuilt into its member first
    (`seal_below`); one that is removed or replaced drops what changed inside. An index read from an overlay is dropped
    when the member changes and when the batch ends. `seal_below` and the commit share `fold_changed`, which looks at the
    levels again after each fold, because a fold makes the level around it: the list of levels taken once missed a level
    with no change of its own, and a write three levels down was lost behind a success. `put_rebuilt` on a missing
    member and a commit whose change did not arrive are errors.
15. **`begin` on a single stream is `OperationUnsupported`** and fails the item, as §1.5.10 wants of the primitives. A `keep_dir`
    item opens a root too, so `mkdir -p x.gz/` fails rather than saying nothing happened, and so a failed rebuild turns it
    into a failed item.
16. **Implicit folders.** One that loses its last member is written as a record with the clock's time, so a listing does
    not change under the user (P-ARCH-2 compares with `MemoryVfs`). `set_times` and `set_attrs` make one explicit.
17. **A failed second phase.** A source that changed while the rebuild ran stays, and its item fails "changed during the move;
    the copy was kept" (length, time, identity against what was copied). An empty folder of a move into an archive waits
    for the rebuild too. When a rebuild is lost, a `move` item keeps the error and a waiting `remove_dir` stays `not_run`.
18. **Links.** A target that is not text, is empty or is past 4,096 bytes is `InvalidName` at `create_symlink`: no reader would
    give it back. A link whose target was too long to keep can therefore be moved inside an archive but fails an item
    when copied to another. `Attrs::Dos` of a member with a Unix mode maps `R` to the write bits, so a copy from Windows
    does not fail on a new member, which starts with a mode.
19. **Times of the archive file.** §1.5.10 keeps permissions (the rule of `LocalWriter`) and no times, and §1.5.17 needs a new
    stamp, so the file gets a new modification time. Owner, ACLs and extended attributes of the file are not carried over:
    `LocalWriter` does not, for any file. A rename replaces a name, so a second hard link to the archive keeps the old
    bytes, and a read-only archive in a writable folder is rewritten and stays read-only: the folder decides, as for any
    local write (M1). On Windows that rename fails `permission_denied` and the archive stays. A symbolic link stays.
20. **`MemoryVfs::clear_faults`** also clears the stream faults, as its text always said.
21. **Windows.** Reasoned, not run: a rebuild closes every handle it opened on the container before `commit` (`Rebuild::run`
    owns them; §1.5.19 W5a note 1 keeps none between calls), and `LocalWriter` closes its own before the rename. A member
    reader the caller still holds on a plain tar is a handle on the file: Rust opens with `FILE_SHARE_DELETE` and its
    `fs::rename` asks for POSIX semantics, so on NTFS the rename works and the reader keeps the old bytes; on FAT and old
    systems it fails `permission_denied`, the temp file goes with the writer, and the archive stays. The Windows-target
    clippy covers the code; the rename itself needs the maintainer's CI run.

22. **Green is claimed per part**, as W5a's note 21: W5b's commits are grouped by subject, and an intermediate one is not
    claimed to build or pass on its own. Rebuilds have no gate (§1.5.16), so none was measured.
23. **The writer's lock** (D-157, `local/lock.rs`). Beside the file a link names, so all spellings share it. It is taken with
    `try_lock`, compared with the path afterwards, as in the cache, and removed by its holder under the lock, after it
    compared the path once more: what stands at the name by then is not the holder's to remove. A lock file a process
    cannot open is busy, and the message names it, since nothing but the user removes a folder or another user's file at
    that name. A folder that takes no new file or a file system without locks gets no lock, and the write gives the error it
    always gave; on such a file system the empty lock file stays. Windows was reasoned, not run: `LockFileEx` on a sibling
    keeps nobody off the archive and conflicts between two handles of one process as `flock` does, a name marked for
    deletion refuses opens, which reads as busy, and the OS drops the lock with the process. A virus scanner or an indexer
    can hold a new file for a moment: on Windows the removal of temporary and lock files and the rename of `commit` retry
    for about a third of a second over `ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`
    (`platform::remove_file`, `platform::rename`). `racing_writers_lose_no_update`, `the_lock_file_lasts_as_long_as_the_batch`,
    `dead_temporary_files_go_and_nothing_else_does`, `a_second_writer_is_refused_while_a_batch_runs` and
    `no_temporary_sibling_stays` have run under wine at best, so their first real run on Windows is the CI of this release.
24. **Temporary files** (D-158). A temporary file is created with the default mode, read, and made `0600` before a byte goes
    in; `commit` sets the replaced file's mode or that first mode: this keeps M1's result without reading the umask. Windows
    has no such mode: a file's ACL comes from its folder. **A kill leaves files.** A kill, or a second Ctrl-C (D-159), during a
    rebuild leaves `.<name>.hikasa-tmp` and the lock file next to the archive, which stays as it was. The next rebuild of
    that archive removes them. A first Ctrl-C cancels, and the rebuild removes them itself. A temporary file is locked before
    anything else happens to it, and given up for the next name if a rebuild got there first. On Unix the writer holds it
    through the rename, so a live writer's file cannot be taken for a dead one's. **On Windows the handle closes before the
    rename**, which leaves one moment in which a rebuild of the same file can remove the temporary file of a plain write,
    and that write then fails with the target as it was. What a rebuild looks for is listed from the folder, exact names only.
    On a file system that cannot show mode `0600` (FAT, some mounts) a dead writer's file does not look like one, and stays.
25. **A path that comes free.** A member refused for a conflict stays, as every refused member does, so after `rm` of the
    folder that hid it, it reads at that path: `rm` reports done and a warning says so. Dropping it instead would break "a
    rebuild keeps what it did not touch"; the warning is `archive_member_refused`.
26. **Times out of range.** `set_times` with a time the format cannot give back (before 1970 in a tar or a 0x5455 field, past
    2106 in that field) stores the nearest it holds, as §1.5.8 says, with one `capability_missing` warning. A touch keeps the
    access and change times of a 13-byte local 0x5455 field.
27. **The ratio of a result** only refuses a rebuild that added content. A remove or a rename cannot fail on it: without
    the incompressible part an archive's ratio rises, and the operation is what makes it smaller. It warns, since reads now
    need a higher `ratio_max`. Note 13 stands for what is added; a nested archive rebuilt into its parent counts as added.
28. **Raw names in a move.** A zip name that is not UTF-8 is another code page, whose CP437 text is only a reading: a
    rename keeps the bytes of the components it leaves alone and writes new ASCII around them, without the UTF-8 flag.
    New text that is not ASCII cannot share a name with such bytes, so then the whole name is the UTF-8 of what the
    listing shows, with the flag: the one case where those bytes go.
29. **A signature of the central directory** (`PK\x05\x05`) signs what a rebuild changes, so it goes, with a
    `capability_missing` warning. **A volume label** of GNU tar has an empty size field, which the crate does not read as a
    number: `tar_walk::entry_size` reads it as 0, and the label stays.
30. **A panic in a primitive** of a change outside a batch ends its batch (`AbortUnlessEnded`), which would otherwise stay open
    and busy for good. New for tests and diagnostics: `ArchiveCounters::rebuilds`, `Service::archive_counters`.

*Hooks for W6.* `ops::execute` already gives W6's `OpPlan` batches, the two-phase move and item results. The backup of
§1.5.17 goes in `ops/roots.rs::finish`, before `end(root, Commit)`. A plan that waits on a confirmation should re-plan or
carry its stamp: note 7. Sources of `OpPlan` items on one thread: note 6. `Fate`, `Overlay` and `Origin` are crate-private, and
`Vfs::create_archive`, `begin`, `finish_with` are the whole surface.

### 1.6 W6 — Folder operations and sync engine

**Ids.** FC-003, FC-012 (the options; glyphs W9), FC-029 (b, c), FC-033, FC-040 (CRC on demand), FC-052, FC-060
(selection engine), FC-061 (Find Filename), FC-062 (Open With expansion, Compare To), FC-070, FC-071, FC-072, FC-073,
FC-074, FC-076, FC-077, FC-078, FC-100 (operation log model), FS-001, FS-002, FS-004, FS-005 (the engine; the view is
W9), FF-002, CLI-003 (`--sync`, `--nobackups`), CLI-G03 (`folder sync --gui`), plan A5 (attribute and Unix-type
filters), T12, ARCH-101, gate G7. Decisions D-161…D-192. Out of scope: every view, list and tree of W9 (selection by
pointer, preview lists, Folder Sync view, columns, Legend, browse dialog), FS-003 Sync Memory (P1), FC-075 Attributes
(P1, §1.6.23), FC-082 re-stat of a whole view (P1), ACL and 8.3 options (D-59), FC-081 (D-59).

**Files.** `core/hikasa-core/src/` (`job.rs`, `warning.rs`); `core/hikasa-vfs/src/` (`vfs.rs`, `entry.rs`, `router.rs`,
`memory.rs`, `local/mod.rs`, `local/unix.rs`, `local/windows.rs`, `archive/mod.rs`, `archive/cache.rs`,
`archive/index.rs`, `ops/mod.rs`, `ops/plan.rs`, `ops/copy.rs`, `ops/rename.rs`, `ops/remove.rs`, `ops/touch.rs`,
`ops/roots.rs`, new `ops/backup.rs`, new `ops/exchange.rs`); `core/hikasa-formats/src/regex_engine.rs`;
`core/hikasa-folder/src/` (`rules.rs`, `resolve.rs`, `scan.rs`, `stats.rs`, `model/mod.rs`, `model/node.rs`,
`model/store.rs`, `lib.rs`, new `refresh.rs`, `content.rs`, `select.rs`, new `ops/` with `mod.rs`, `build.rs`,
`conflict.rs`, `confirm.rs`, `rename.rs`, `copyto.rs`, new `sync.rs`); `core/hikasa-session/src/`
(`settings/options.rs`, `sessions/file.rs`, `sessions/scope.rs`, `host.rs`, `log.rs`, `api/mod.rs`, `api/compare.rs`,
`api/requests.rs`, `api/dto.rs`, `api/human.rs`, `api/vocab.rs`, `api/gui.rs`, `api/sessions.rs`, new
`api/folder_ops.rs`, new `launcher.rs`, and `core/hikasa-session/catalogue/commands.toml`); `cli/hikasa-cli/src/`
(`args.rs`, `cmd/folder.rs`, `cmd/session.rs`, new `cmd/folder_op.rs`, new `cmd/folder_sync.rs`, new `confirm.rs`,
`main.rs`); `gui/hikasa-gui/src/` (`folder_view.rs`, `commands.rs`, new `op_dialogs.rs`, `ui/dialog.rs`, `ipc.rs`,
`main.rs`); workspace `Cargo.toml`, `Cargo.lock`, `core/hikasa-session/Cargo.toml`, `core/hikasa-folder/Cargo.toml`,
`core/hikasa-session/tests/architecture.rs`; `help/` (§1.6.24); tests of §6.6.

#### 1.6.1 Parts (D-161)

W6 is one wave in four sequential parts, one implementer each; a part starts when the previous one is committed on
`feat/m2` and ends with the tree green. W7 may run beside it: the only files both touch are `SCHEMAS` in `api/dto.rs`
and `args.rs`, and each appends.

| Part | Content | Sections | Acceptance | Leaves working |
|---|---|---|---|---|
| **W6a Foundations** | batch plans, stamps, exchange, durability, fence, backups in `hikasa_vfs::ops`; `extract_to_cache`; the A5 filters, FC-012 keys, FF-002, Ignored marks with their flags (`FolderRuleArgs`, §1.6.17); subtree refresh; contents on demand | §1.6.2–§1.6.8, `extract_to_cache` of §1.6.18 | A6.1…A6.13, A6.35 | `vfs` commands as before, faster; `folder compare` with its new flags; no new subcommand. Everything else is additive API with tests |
| **W6b Operations** | selection, planning, confirmation, delete, rename, Disable editing, Service, `folder op`, `hikasa.folder_op.v1`, `hikasa.folder_contents.v1`, Exclude, Open With, the log model | §1.6.9–§1.6.13, §1.6.15 (specs), §1.6.16–§1.6.19, §1.6.21 | A6.14…A6.25, A6.35 | every file operation headless, with `--dry-run`; the window unchanged |
| **W6c Sync** | matrix, presets, overrides, sync plan, `SessionRules::Sync`, `folder sync`, `session run` of a sync, `--sync` and `--gui` up to IPC, G7 | §1.6.14, §1.6.15 (rules), §1.6.17 (sync), §1.6.22 | A6.26…A6.32, A6.35 | sync headless and from a session |
| **W6d GUI hooks** | confirmation and delete dialogs, four commands, log and notices, refresh in the view, the child result of FC-029(c); docs, 0.1.92 | §1.6.20, §1.6.24 | A6.33…A6.35 | a selected row can be copied and deleted in the window; the rest of the Actions menu stays `planned` until W9 |

Each cut is additive: W6a adds nothing a user meets but flags, W6b nothing but subcommands, so the tree stays green and
shippable after each. The order is forced: W6b's plans need W6a's batch, W6c's sync plan is built by W6b's builder, W6d
shows W6b's plans. Green is claimed per part (§1.5.19 W5a note 21).

#### 1.6.2 Batch plans and execution (D-162, D-163, D-165)

`hikasa_vfs::ops` stays the only place that writes. A folder operation is **not** a sequence of `ops` calls: a sync
copies both ways and deletes, and two `execute` calls would mean two rebuilds and two backups of one archive. It is one
`ops::Plan` of a new kind, executed once.

```rust
// hikasa_vfs::ops::plan (additive)
pub enum OpKind { Copy, Move, Remove, Mkdir, Touch, Batch }                  // as_str "batch"
pub enum PlanOptions { /* … */ Batch(BatchOptions) }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BatchOptions {
    pub preserve: Preserve,   // metadata of copies; moves keep Preserve::default() as in W3
    pub links: LinkPolicy,
    pub moves: bool,          // `move`/`moveto`: the folder items of the plan are sources of a move (two-phase, D-148)
}
pub enum Action { /* … */ Exchange }                                          // as_str "exchange"; needs_absent_target false
pub enum SkipReason { /* … W3 */ Declined, Filtered, CompareError, FollowedLink, ReadOnlySide, Capability,
                      DuplicateTarget, NoMatch, SameName }                    // declined, filtered, compare_error, followed_link,
                                                                              // read_only_side, capability, duplicate_target,
                                                                              // no_match, same_name

/// What a plan saw of an item: execution compares it, so a plan that waited on a confirmation stays honest (D-163).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStamp { pub kind: EntryKind, pub size: Option<u64>, pub mtime: Option<FileTime> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Expect { pub source: Option<ItemStamp>, pub target: Option<ItemStamp> }

pub struct PlanItem {
    /* … W3 fields unchanged … */
    #[serde(default, skip_serializing_if = "Option::is_none")] pub expect: Option<Expect>,
    /// Where the replaced target goes first (§1.6.5). `None` for members: their archive is backed up whole.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub backup: Option<Spec>,
    /// The times of a `touch` item of a batch; other plans use `TouchOptions`.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub times: Option<SetTimes>,
    /// The source is a link the comparison followed (FC-013): stat and read through it.
    #[serde(default)] pub follow_source: bool,
    /// Answered yes to `read_only`: a DOS `R` on the target is cleared just before it is replaced or removed.
    #[serde(default)] pub clear_read_only: bool,
    /// A `trash` item that may become a permanent delete when the recycle bin fails (§1.6.12).
    #[serde(default)] pub trash_fallback: bool,
}
impl PlanItem { pub fn planned(action: Action, kind: EntryKind, source: Option<Spec>, target: Spec, depth: u32) -> PlanItem; }

/// What `execute_with` adds to `execute`. `Default` is `execute`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecOptions {
    pub fence: Option<Fence>,                     // §1.6.4
    /// Canonical outermost archive file → its backup (§1.6.5). Filled by the planner, so a dry run shows them.
    pub archive_backups: BTreeMap<String, Spec>,
}
pub fn execute_with(src: &dyn Vfs, dst: &dyn Vfs, plan: Plan, options: &ExecOptions, ctl: &Ctl<'_>) -> OpReport;
pub fn execute(src: &dyn Vfs, dst: &dyn Vfs, plan: Plan, ctl: &Ctl<'_>) -> OpReport;   // execute_with(.., &Default::default(), ..)
```

`PlanItem::seen` stays private and unserialized. Folder plans leave it `None` (the model holds no `FileId`), so the
folder re-check falls back to "every planned folder above is still a folder" (§1.3.12).

**Batch semantics in `Exec`.** A batch is a tree operation (`tree_op`: the cut of §1.3.12 applies; roots are depth 0, so
a failed folder cuts its own subtree only). `Exec::preserve` = `BatchOptions::preserve`; `Exec::overwrite` = `None`: a
replacement was decided at planning and confirmed, so `newer` is not decided again, the stamp does that job.
`create_parents` is false: the plan holds a `create_dir` for every missing folder. Every `matches!(options,
PlanOptions::Move(_))` of `ops/copy.rs` becomes `exec.moving()` (`Move(_)`, or `Batch` with `moves`). `run_rename`'s
fallback expansion takes `MoveOptions::default()`. `run_touch` takes `item.times`, then `TouchOptions`; a `touch` item
of a batch always has `times` (the builder sets it, a debug assertion says so). `Exec::total` and `Plan::bytes` count
both sizes of an `exchange`. `ops::execute` runs on **one thread**: the batches of §1.5.19 W5b note 6 belong to it. The
Service never hands execution to a pool, and the GUI runs it in one job thread per view.

**Order of checks in `run_item`** (the first that fails decides; steps 2 and 3 are new):

1. `item.skip` → `Skipped`.
2. Fence (§1.6.4).
3. `item.expect`. `source` and `target` are compared with an `lstat` (`stat` through the link with `follow_source`):
   same `kind`; for `File` and `Symlink` same `size` where `Some` and the same `mtime` where `Some`, exactly; for `Dir`
   the kind only (a folder's time moves with its children). A gone item, or any other value → `InvalidTarget "changed
   since the plan was made"`. `Expect::target` set and the target absent is the same error; `None` asks nothing here,
   and the absent-target check of D-103 stays. The `lstat`s are reused by the copy that follows, so a stamp costs no
   system call beyond W3's re-check.
4. The W3 re-checks, then the action.

**Why stamps, not a re-plan (D-163).** Hook of §1.5.19 W5b note 7: a plan that waits on a confirmation re-plans or
carries a stamp. Plans are built from the model (§1.6.10), so a re-plan would repeat the same stale facts unless it
re-scanned, and a re-scan moves the items under a confirmation the user already answered. A stamp costs nothing and
refuses exactly what changed: a file modified after the comparison is never overwritten or deleted on the strength of an
old row, and the item says so. Other items go on (ARCH-101). For archives the stamp is the member's: `begin` reads the
archive's own stamp afterwards (D-157), so a change to *other* members since the plan is kept by the rebuild and cannot
be lost. On Windows, a directory listing can show an out-of-date size or time for a file with an open handle or a hard
link: the item then fails `changed since the plan was made`, which is the safe side; a refresh clears it.

**Exchange (D-165).** `Action::Exchange` has `source` = the left file or link, `target` = the right one, `replaces =
true`, both stamps. FC-071's "swap both sides" must not lose either side at any step, across file systems and into
archives, so `ops/exchange.rs` makes both copies first:

| Step | Call | On failure |
|---|---|---|
| 1 | copy `source` to `T1`, a free hidden sibling of `target` (`.hikasa-xchg[-n]`), metadata applied | remove `T1`; both sides untouched; item fails |
| 2 | copy `target` to `T2`, a free hidden sibling of `source`, metadata applied | remove `T1`, `T2`; both sides untouched; item fails |
| 3 | `rename(T1, target, replace = true)` | remove `T1`, `T2`; both sides untouched; item fails |
| 4 | `rename(T2, source, replace = true)` | `target` holds the new content; `T2` is left in place and the item fails with `InvalidTarget "changed during the move; the copy was kept"` naming `T2`: the other side's old content is that file |

Links are copied as links (`create_symlink`); steps 3 and 4 rename them like files. No backup is taken: nothing is lost.
A pair of folders is not an item: the builder exchanges their contents row by row, and an orphan inside is an ordinary
`Action::Move` to the other side. Cancellation is checked before steps 1, 2 and 3; after step 3 the item runs to its
end. `Roots::touch` opens the root of either side, each as a target.

**Exchange and archives.** Both sides in one archive: one batch, one rebuild, atomic. A plain file against a member:
step 4 joins `exec.deferred` and runs after the rebuild, as a move's removal does (D-148); a failed rebuild removes `T2`
and leaves the plain file. Members of **two different archives** cannot be exchanged safely: when the second rebuild
fails the first has already lost its old member. The builder gives such an item the error `unsupported` ("exchanging
between two archives"): copy, or move, instead.

#### 1.6.3 Durability and the copy path (D-164)

W3's `LocalWriter::commit` calls `sync_all` on the file and on its folder, one pair of `fsync` per file. Copying 10,000
small files that way took 26 × a plain copy on the reference machine, so G7 cannot be met by tuning anything else;
macOS's `F_FULLFSYNC` and Windows' `FlushFileBuffers` are slower still. What a sync protects differs by case, so the
case decides:

```rust
// hikasa_vfs::vfs (additive)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Durability {
    /// `sync_all` of the file, then of the folder after the rename (M1, saves, settings, rebuilds, moves).
    #[default] Full,
    /// `sync_all` of the file only: a replaced file is either the old or the new one after a power cut.
    FileOnly,
    /// Neither: a new file that nothing depends on yet.
    None,
}
pub struct WriteOptions { /* … W3 … */ pub durability: Durability }       // literals use `..Default::default()`
/// Metadata a back-end can apply on the open handle, before the file gets its name.
#[derive(Debug, Clone, Copy, Default)]
pub struct HandleMeta { pub modified: Option<FileTime>, pub created: Option<FileTime>, pub mode: Option<u32> }
pub trait Vfs: Send + Sync {
    /// A copy of the regular file `from` to `to`, which does not exist, with `meta` applied, or `Ok(None)` when this
    /// back-end has no such path. Never replaces: a target that appeared meanwhile stays, and this is `AlreadyExists`.
    fn copy_new_file(&self, _from: &Spec, _to: &Spec, _meta: &HandleMeta, _cancel: &CancelToken)
        -> Result<Option<u64>, VfsError> { Ok(None) }
    /// Makes what happened in the folder `dir` durable, best effort: names, not contents (§1.6.25 note 34).
    fn sync_folder(&self, _dir: &Spec) {}
}
```

| Item (`ops/copy.rs::copy_content`) | Durability | Path |
|---|---|---|
| `copy`: target absent | `None` | `copy_new_file`, else `copy_file_fast`, else stream |
| `copy`: `replaces` | `FileOnly` | `copy_file_fast`, else stream; with `backup` always stream (§1.6.5) |
| `move` (either) | `Full` | a source is removed on the strength of the copy, so the copy must be on disk first |
| `exchange` | `FileOnly` on `T1`, `T2` | `copy_file_fast`, else stream |
| `open_write` outside `ops` (save, `--output`) | `Full` | unchanged |

`LocalVfs::copy_new_file` (files ≤ `FAST_COPY_MAX`; larger ones `Ok(None)`, for G4): the temporary sibling is opened
`create_new` with the final mode already on Unix (`OpenOptionsExt::mode(0o600)`, then `File::set_permissions` to
`meta.mode` before the rename, which replaces D-158's read of the first mode for this path only: `meta.mode` is the
result), `std::io::copy`, `File::set_modified` (and `FileTimesExt::set_created` where `SET_CREATED`), close, a rename to
the target that cannot replace (§1.6.25 note 8). So the file never shows under its name with a wrong time or mode, and a
file another program creates there meanwhile stays. Windows closes the handle before the
rename, as elsewhere, and the DOS `R` bit is applied after it by the usual step. No such path when `preserve.attrs` is
off (the result needs the default mode, which only the D-158 sequence gives without reading the umask). `ops` skips the
metadata step of §1.3.4 after `Ok(Some(_))`, except `R`.

Measured with this sequence on `/var/tmp` (Linux, 10,000 files of 4 KiB in 100 folders, three runs each, no pool): plain
copy keeping times 240 ms, W3 sequence without `fsync` 425 ms (1.8 ×), this path 340 ms (1.45 ×), with both `fsync` 6.2
s (26 ×). **A power cut just after a copy can lose files the copy created; it does not lose a file that existed**: what
replaces it was synced before the rename, and no source is removed before its copy is on disk, with the name of the copy
and of each folder the operation made for it (§1.6.25 note 34). That rests on the file system's rename being atomic, as
everything here does. `docs/11` says so. `vfs cp` takes the same path, since it runs the same code.

#### 1.6.4 Fence (D-166)

A mirror must never delete outside the base folders, whatever the tree holds. The planner guarantees that, and execution
refuses it again, because the planner works from a model that may be old.

```rust
// hikasa_vfs::ops
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fence { pub bases: Vec<Spec>, pub writes: bool }
/// True when `spec` lies strictly below `base`: `Spec::parent` is walked up (it leaves an archive through its root,
/// §1.5.2) and canonical texts compared, case-folded when the back-end is not `CASE_SENSITIVE`. Never true for `base` itself.
/// Texts only: what the tree holds is `run_item`'s, below.
pub fn within(vfs: &dyn Vfs, base: &Spec, spec: &Spec) -> bool;
```

`run_item` refuses, with `InvalidTarget "is outside the base folders"`: every `remove`, `remove_dir`, `trash` target;
the source of a `move`, `rename`, `exchange`; and with `writes`, every target. `copyto`/`moveto` set `writes = false`
(their target is elsewhere, by request); every other folder operation and every sync set `true`. A base itself is never
removed. A backup with `naming = "folder"` lies outside the bases on purpose: backups are not items, and the fence does
not see them.

**The tree (§1.6.25 note 33).** A text can be below a base while the path leads elsewhere, because a folder of the base was
moved out and left as a link after the plan was made; every stamp holds then, as it is the same file. So each spec the
fence looks at is walked too: from the base down to the spec's folder, one `lstat` a level, at any depth, no component may
be a link, or `InvalidTarget "is a link below a base folder, where the plan saw none"` names it. A link at or above a base
is the user's choice and stays. The item itself is no component: a link to remove or replace is that link, and rule 2
below stands. A folder that is not there ends the walk, and the item fails as it would. What is removed, moved or replaced
is looked at each time; a new file takes what was seen of its folder in this batch, for G7, until an item of the plan makes
a link or renames something. Members are not walked: a member cannot be swapped for a link. The file of an archive is: an
archive behind a link below a base would be rewritten outside it.

**What the fence cannot see, the planner refuses (normative for every folder plan):**

| # | Case | Rule |
|---|---|---|
| 1 | A row below a folder the comparison **followed** (FC-013), on the side being written or deleted | never touched: the path leads outside the base. `skip`, reason `followed_link` |
| 2 | A followed link itself | `remove`, replace: act on the link (D-94 rule 1 and 4) |
| 3 | Rows in state Unknown or Error, and rows below a folder that failed on either side | never written, never deleted: a failure is not a difference (D-19). `skip`, reason `compare_error` |
| 4 | A folder not scanned (cancelled comparison, auto-scan off) in a **sync** | as rule 3. In a `folder op` the user named it: §1.6.10 rule 6 |
| 5 | An empty or incomplete source | §1.6.14 "Empty sources" |

Rule 1 refuses what W3's D-94 rule 5 refuses for copies: a link is never written through. Execution has the D-103 parent
re-check and the walk above for what changes after planning, which narrow the race and cannot close it (§1.3.3 rule 3):
a link swapped in between the `lstat` and the call is not seen.

#### 1.6.5 Backups (D-167, D-168)

Option `[backups] back_up_before_copy` (OPT-008), `--backup`, `--no-backup`, legacy `--nobackups`. **A backup keeps what
a copy or move replaces.** Deletes have the recycle bin: `Action::Remove` is not backed up (§1.3.10 named it as the
recycle point). A `.bak` left by a delete would be an orphan of the next mirror, deleted and backed up in its turn, for
ever.

```rust
// hikasa_vfs::ops::backup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupStyle { NameTilde, StemTilde, NameBak, TildeName, StemBak, StemBakExt, Folder }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPolicy { pub style: BackupStyle, pub folder: Option<Spec> }   // `folder` for Folder
impl BackupPolicy {
    /// The backup of `item`: a sibling, or `<folder>/<rel>`; `rel` is the path below the base (`None`: the file name,
    /// as a save of W10 uses it). `Folder` without a folder → `InvalidTarget "needs a backup folder"`.
    pub fn target(&self, vfs: &dyn Vfs, item: &Spec, rel: Option<&RelPath>) -> Result<Spec, VfsError>;
}
/// A copy of `item` at `backup`, parents created, replacing a backup that is there (atomic, `Durability::FileOnly`).
pub fn write_backup(vfs: &dyn Vfs, item: &Spec, backup: &Spec, ctl: &Ctl<'_>) -> Result<(), VfsError>;
/// The names of one plan's backups.
#[derive(Debug, Default)] pub struct BackupNames { /* … */ }
impl BackupNames {
    /// Keeps a path the plan writes, removes or reads from every backup of the plan: the planner calls it for each target
    /// and source of the plan, first.
    pub fn reserve(&mut self, vfs: &dyn Vfs, spec: &Spec);
    /// The backup of `item` in this plan, numbered around what is taken. A folder at that name → `IsADirectory`.
    pub fn assign(&mut self, policy: &BackupPolicy, vfs: &dyn Vfs, item: &Spec, rel: Option<&RelPath>) -> Result<Spec, VfsError>;
}
```

`hikasa_session::settings::BackupNaming` maps one to one to `BackupStyle` (`impl From`); `Service` builds the policy; a
relative `[backups] folder` is `invalid_input`, an empty one with `folder` too. **The core owns the name, the session
the choice.**

| `naming` | `abc.txt` | `Makefile` | `.bashrc` | `a.tar.gz` |
|---|---|---|---|---|
| `name_tilde` | `abc.txt~` | `Makefile~` | `.bashrc~` | `a.tar.gz~` |
| `stem_tilde` | `abc~.txt` | `Makefile~` | `.bashrc~` | `a.tar~.gz` |
| `name_bak` (default) | `abc.txt.bak` | `Makefile.bak` | `.bashrc.bak` | `a.tar.gz.bak` |
| `tilde_name` | `~abc.txt` | `~Makefile` | `~.bashrc` | `~a.tar.gz` |
| `stem_bak` | `abc.bak` | `Makefile.bak` | `.bashrc.bak` | `a.tar.bak` |
| `stem_bak_ext` | `abc.bak.txt` | `Makefile.bak` | `.bashrc.bak` | `a.tar.bak.gz` |
| `folder` | `<folder>/<rel>/abc.txt` | same | same | same |

Extensions as D-07: a `.` that is neither first nor last; the last one counts. The `folder` form keeps the path below
the base, a choice of this contract (docs/15 says "in this folder"), because a flat folder would let `a/x.txt` overwrite
the backup of `b/x.txt`.

**Rules.**

- **Plan.** The planner sets `PlanItem::backup` on every `copy`, `move`, `rename`, `link` and `create_dir` with
  `replaces`, outside archives. Names are final at planning, so a dry run shows them and two items can be told apart: a
  name another path of the plan has gets `-2`, `-3` … before its extension (`abc.bak`, `abc-2.bak`), in plan order, for
  `abc.txt` and `abc.doc` with `stem_bak`, for `left` and `right` of one path with `folder`, and for `x` next to a target
  `x.bak`, which is why every path of the plan is reserved first. A name that comes out as the item itself is numbered in
  every style, `folder` included (the item's own folder as the backup folder). On disk one generation is kept: an
  existing file or link at that name is replaced. A folder there fails the item `is_a_directory` at planning.
- **Execution refuses what would lose a file.** `execute_with` checks the plan first, hand-built plans too: a backup that
  is its own item, or the target or source of any item, or another item's backup, or the outermost file of an archive
  whose backup it is. The item fails `InvalidTarget` (`is the item it backs up`, `is a path of another item of the plan`)
  and the rest are `not_run`: nothing has run.
- **Execution, the order matters.** `copy_content` with `backup` always streams: the new content goes to the temporary
  sibling, and just before `commit`: `rename(target, backup, replace = true)` (parents created for `folder`), then
  `commit`. A failure of the stream leaves target and backup as they were. A failed `commit` renames the backup back; if
  that fails too the item's message names the backup. No extra copy, no extra disk, and the target is missing for one
  rename only. `CrossDevice` or `OperationUnsupported` from that rename (a `folder` on another disk): `write_backup`
  while the target is intact, then `commit`. **A failed backup fails the item and the target stays:** nothing is
  replaced without its backup. `link` and `create_dir` go through `replace_with` (§1.3.12), whose hidden sibling becomes
  the backup instead of being removed.
- **Archives (D-168).** A `replaces` item inside an archive gets no `backup`: **one backup of `root.outermost_file()`
  per operation** replaces per-member backups, as §1.5.17 proposes, because members cannot be renamed aside cheaply and
  the archive holds them all. `ops/roots.rs::finish` runs it in both loops, for the target roots and the source-only
  roots, just before `end(root, Commit)`: when `options.archive_backups` names `root.outer` and the root has a `Done`
  item: a copy of the file's bytes, through a link that leads to it (`write_backup` keeps a link as a link, and a link
  would lead to the rewritten file). A failure ends that root with `Finish::Abort` and fails its done items with that error, so an
  archive is never rewritten without its backup. A cancelled root has no backup. `write_backup` runs under the writer's
  lock (D-157), which only keeps other writers out: the file reads as usual. The planner fills the map for every
  outermost file an item of the plan writes, removes or moves out of, whether or not that item replaces: a rewrite
  touches the whole file. An outermost file that is not below a base (the base is the archive) takes `rel = None`. Its
  size is reported in `Phase::Writing`.
- `create_dir` of `Folder` parents is not an item and is not counted. Backups are not comparison rows: a later refresh
  shows them as the orphans they are.

#### 1.6.6 Rules and scan (D-169…D-172)

Every key below is sparse and additive: `FOLDER_RULES_SCHEMA_VERSION` stays 1, defaults reproduce M1's results, and the
folder goldens do not change.

```rust
// hikasa_folder::rules (additive)
/// `attrib:(+|-)<letters>…`, FC-053's DOS attribute test. `set`: an item must have them all; `clear`: none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct AttribFilter { pub set: DosAttrs, pub clear: DosAttrs }
bitflags! { pub struct UnixTypes: u8 { BLOCK = 1; CHAR = 2; LINK = 4; FIFO = 8; REGULAR = 16; SOCKET = 32; } }   // letters b c l p r s
/// `unixtype:(+|-)<letters>…`. `include` not empty: only these; `exclude`: not these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct UnixTypeFilter { pub include: UnixTypes, pub exclude: UnixTypes }
impl AttribFilter { pub fn parse(text: &str) -> Result<AttribFilter, FolderError>; }     // serialized as this text
impl UnixTypeFilter { pub fn parse(text: &str) -> Result<UnixTypeFilter, FolderError>; }  // serialized as this text
pub struct OtherFilters { /* … */ pub attrib: Option<AttribFilter>, pub unix_type: Option<UnixTypeFilter> }  // `false` = off, D-90
pub struct Handling { /* … */ pub auto_scan_subfolders: bool /* true */, pub expand_on_load: bool /* false */,
                      pub expand_differences_only: bool /* false */ }
pub struct FolderRules { /* … */
    pub ignored: Vec<String>,                       // FC-033, §below
    pub format_overrides: BTreeMap<String, bool>,   // FF-002: format id → enabled
}
pub struct FolderRulesPatch { /* … */ pub attrib_filter: Option<Change<AttribFilter>>,
    pub unix_type_filter: Option<Change<UnixTypeFilter>>, pub auto_scan_subfolders: Option<bool>,
    pub expand_on_load: Option<bool>, pub expand_differences_only: Option<bool>,
    pub ignored: Option<Vec<String>>, pub format_overrides: Option<BTreeMap<String, bool>> }
// hikasa_vfs::entry (additive; every struct literal of `Entry` is inside hikasa-vfs)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum SpecialKind { Block, Char, Fifo, Socket }
pub struct Entry { /* … */ pub special: Option<SpecialKind> }     // `Some` only with `kind == Other`
// hikasa_folder::NodeFlags
NodeFlags::IGNORED = 4096                                          // name "ignored"; FLAG_NAMES grows to 11
```

**Attribute and Unix-type filters (A5, D-169).** The filter vocabulary of SCR-001 (`filter attrib:…`, `filter
unixtype:…`) is P0 while FC-053 is P1, so the two tests are in M2; the content filter stays P1. Grammar: one or more
groups of a sign and letters: `+rh-s`; `attrib` uses `DosAttrs::parse_letters`, `unixtype` the letters `b c l p r s`; a
letter on both signs, a group without letters or an unknown letter → `InvalidRules` (CLI: `invalid_input`, exit 100);
`none` (script, flags) is `Change::Off`. TOML: `[rules.filters.other] attrib = "+rh-s"`, `unix_type = "-lp"`, `false`
off (`off_or`, D-90).

| Filter | Applies to | Facts |
|---|---|---|
| `attrib` | regular files, as size and date filters (a folder stays, or the subtree would vanish) | `Attrs::Dos` only. On a back-end without DOS attributes it has no effect, with one `capability_missing` warning per side |
| `unix_type` | everything but folders; `d` is not a letter | `File` → `r`, `Symlink` → `l` (an unfollowed link; a followed one is its target's type), `Other` → `Entry::special`. `Other` without `special` matches no letter: `+p` hides it, `-p` keeps it |

`ResolvedRules::other_filtered` gains `special` and tests both, so the scan's two call sites and INV-FOLDER-7 stay one
predicate (T11). `Entry::special`: `LocalVfs` on Unix from `FileTypeExt`; `ArchiveVfs` from tar type flags 3, 4, 6;
`MemoryVfs` from `MemMeta::special`; `None` on Windows and in zip. Name-only entries (D-45) are not tested, as the other
filters. The filtered row is teal as ever (FC-054).

**Folder handling (FC-012, D-170).** `expand_on_load` and `expand_differences_only` change no result:
`hikasa_folder::expanded_on_load(read: &TreeRead, handling: &Handling) -> Vec<NodeId>` is what the view opens (and
`--expand-all` wins). `auto_scan_subfolders` keeps its M1 result when true; `scan_orphan_folders` keeps its M1 meaning
(any orphan folder), which contains FC-012's *top-level orphan*:

| `auto_scan_subfolders` | `scan_orphan_folders` | Scanned |
|---|---|---|
| true | true | everything (M1) |
| true | false | folder pairs; orphan folders stay `NotScanned` (M1 `--no-scan-orphans`) |
| false | true | the bases, and orphan folders directly in a base with their subtrees (FC-012's "top-level") |
| false | false | the bases only |

A folder not scanned is `NotScanned`, state Unknown: its verdict is `incomplete` (exit 100 unless differences were
found), as D-19 asks of anything unknown. It is scanned by a refresh of its own (§1.6.7), which is what the view does on
Expand. Glyphs are W9. Flatten ignores both keys. CLI: `--no-auto-scan`, `--expand-on-load`, `--expand-differences`.

**Ignored marks (FC-033, D-171).** `FolderRules::ignored` is a list of base-relative paths, `/`-separated, in the
spelling of `open_archives`: `a/b.txt`, `a` (a folder and all below it), and an exception `!a/b.txt`. A path whose first
name starts with `!` is written `./!x`; an exception needs no such thing (`!!x`). **The longest path that names a node or an
ancestor decides.** Keys go through `path_key`, so case follows the comparison's alignment. The
flag is set in `plan_pair` and inherited from the parent, unless the path is an exception. A flag never changes
`CompareState`, `diffs` or content results: `TreeRead::category` (via `own_category_of`) maps `LeftNewer`, `RightNewer`,
`Different`, `LeftOrphan`, `RightOrphan` of an `IGNORED` node to `Same`, so counters, verdict, exit code, display
filters and statistics see *same*, and the columns still show why. `Unknown`, `Error` and `Filtered` stay: a failure is
never hidden (D-19). `FolderStats::ignored` counts them. A sync sees `same` (§1.6.14); a selected ignored row is still
operated on.

```rust
impl FolderModel {
    /// Sets or clears the mark on nodes and everything below them, updates the counters of every ancestor in one mutation,
    /// and returns the new minimal list for the session (a path under an equal ancestor is dropped, `!` only where needed).
    pub fn set_ignored(&self, nodes: &[NodeId], on: bool) -> Vec<String>;
}
```

The list lives in the store from the start of a scan (`Store::ignored`), because `FolderModel::rules` is immutable. CLI:
repeatable `--ignored <PATH>` on `folder compare`, and `folder op ignored`, §1.6.18.

**Enabled formats (FF-002, D-172).** `format_overrides` is a key of the folder rules, plain data, so `hikasa-folder`
needs no format type: `[rules.format_overrides] cpp = false`. `Service::folder_rules` turns it into `EnabledOverrides`,
passes it to `RulesComparator::with_overrides` (the M1 seam) and to `TextCompareRequest::format_overrides` (new,
`choose_format` uses it), so a child session opens with the format its row was compared by. An id the registry does not
know gives one `unknown_config_key` warning and is kept (Q11). CLI: repeatable `--format-on <ID>`, `--format-off <ID>`.
W9 draws the list (bold when different from the global state).

#### 1.6.7 Subtree refresh (D-173)

`FolderModel` was frozen after a run (plan §5.3). After an operation, and for `Open` on an archive row (§1.5.17), a
re-run costs a whole scan to learn a few rows. A refresh re-reads **one folder level** and only descends where a row
changed.

```rust
// hikasa_folder::refresh
pub struct RefreshReport { pub folders: u32, pub rows_changed: u32, pub content_tests: u32,
                           pub renumbered: Vec<(NodeId, NodeId)>, pub complete: bool }
/// Refreshes the folders `nodes` (each a `Folder` node; file nodes count as their parent), on `env`'s pool. A second call while one runs →
/// `FolderError::RefreshBusy`. In a flat comparison → `FolderError::RefreshFlat`: the caller re-runs.
pub fn refresh_into(model: &Arc<FolderModel>, nodes: &[NodeId], env: &FolderEnv<'_>) -> Result<RefreshReport, FolderError>;
impl FolderModel { pub fn open_archive(&self, path: &str); }       // adds to what `opens_archive` consults (Store::opened)
impl TreeRead<'_> { pub fn node_at(&self, path: &RelPath) -> Option<NodeId>; }
```

**Algorithm** (`refresh.rs`, on the `Scan` of `scan.rs`, built from the model's rules and `Store::resolved`, so a
refresh and a scan cannot disagree):

1. Claim the refresh (`AtomicBool`, separate from the one-shot `claim`). A model still scanning is `RefreshBusy`.
2. Remove duplicates from the list; each folder is refreshed once, one level, parents before children.
3. For each folder F: `list_pair`, then the scan's own path: `items`, `pair_items`, `plan`, giving the new rows in final
   order. A listing that fails takes the scan's error path: F becomes `Failed` with its `NodeError` and side flag, the
   other side's entries become *unknown* rows, and the counters follow.
4. Match new rows to old ones by `(kind, left raw name, right raw name)`. Per row:

| Old → new | Result |
|---|---|
| same presence; `size`, `mtime`, `attrs`, link target equal on both sides | **kept**: node, content result, CRC, subtree untouched |
| file-like, a side changed or appeared | new record; quick tests of `plan_pair`; a `ContentTask` when the rules ask for one (its CRC side-table entries go) |
| folder, same presence | kept: its children are not listed (a folder's own time says nothing, §3) |
| folder, a side appeared or went | new record, `NotScanned`, a `FolderTask`: its subtree is scanned anew |
| a file becomes an opened archive (`Store::opened`, or a new stamp below one) | new record, `FolderTask` |
| old row without a new one | dropped with its subtree |
| new row without an old one | as a scan: `FolderTask` for folders, content task for pairs |

5. Same rows in the same order → records are rewritten in place: every id stays. Otherwise a new contiguous block goes
   to the tail of the arena under one write lock, kept folders move there, the `parent` of their children is rewritten,
   and `renumbered` lists `(old, new)`. Node ids of F's block are the only ones that change.
6. Counters: F's `Aggregates` and totals are recomputed from its rows, the difference goes up through `propagate`.
   INV-FOLDER-5 holds after every write.
7. Dropped nodes get `internal::GONE`, their side-table entries are removed, `Store::content_counts` is decreased by
   their results, and `check_tree_invariants` and every read from the root skip them. Arena space of dropped nodes is
   not reused: `Store::garbage` counts it, a view may re-run when it passes half the arena. A long-lived view pays one
   scan, at most, for that.
8. `Ctl` as the scan: cancellation leaves rows `NotScanned`, `complete: false`, verdict `incomplete`, and a mark on what
   was asked for and not reached. A later refresh that covers what is marked restores the verdict; one of another subtree
   does not (§1.6.25 note 21).

A refresh of a folder inside an opened archive lists it through `ArchiveVfs`, whose new stamp brings a new index
(§1.5.17). `FolderComparison::verdict` and `stats` read the live store. `TreeRead::len` counts dropped nodes; nothing
but `check` and benches use it. A folder not scanned (auto-scan off) is refreshed by the same call: every row is new.
**Property P-REFRESH:** after any operation script, `to_canonical` of the refreshed model equals that of a fresh
comparison.

**Which folders.** `FolderOpOutcome::refresh_nodes` (§1.6.16): the parent node of every row a done item changed, from
`ItemMeta::node`, without a path lookup. So copying 10,000 files in 100 folders lists 100 folders twice, and copying one
file into a base lists the base's own level, not the tree.

#### 1.6.8 Contents on demand (FC-029, FC-040; D-174)

```rust
// hikasa_folder::content
pub struct ContentItem { pub node: NodeId, pub path: String, pub kind: ContentKind }
pub struct ContentReport { pub counts: ContentCounts, pub items: Vec<ContentItem>, pub complete: bool }
/// FC-029(b): the content test `method` on the file pairs of `nodes` (a folder: its pairs below it, by §1.6.9 "visible
/// contents"). Results enter the model in place. Runs on `env.pool` with the shared budget (D-25), as a scan's content tasks.
pub fn compare_contents(model: &Arc<FolderModel>, nodes: &[NodeId], method: ContentMethod, env: &FolderEnv<'_>)
    -> Result<ContentReport, FolderError>;
/// FC-040: CRC-32 of the given sides: `Entry::crc32` where a back-end gives one (D-147), else read.
pub fn compute_crcs(model: &Arc<FolderModel>, items: &[(NodeId, Side)], env: &FolderEnv<'_>)
    -> Result<Vec<(NodeId, Side, Result<u32, ErrorReport>)>, FolderError>;
impl FolderModel {
    /// FC-029(c): the result of a child view for its pair. Applied only while both sides still have `stamps` (size and time);
    /// `false` otherwise. A result of a method the row already has is replaced.
    pub fn apply_child_result(&self, node: NodeId, stamps: Pair<ItemStamp>, outcome: ContentOutcome) -> bool;
}
```

`Store::apply_content` today assumes one result per node: it gains the step that takes the old kind out of
`content_counts` and drops an old outcome, so a repeated test counts once. `CONTENT_PENDING` is set per pair while its
test runs and cleared by the outcome; a cancelled run restores the old state, and its report lists only the pairs it
tested. CRCs go to the `crcs` side table
(`EntryView::crc32`) and `SideRec`'s `CRC` bit; a refresh drops them with a changed row. Option
`confirm_content_compare` (OPT-009) is a `Prompt` of W9's dialog; the CLI writes nothing here and does not ask. The
results dialog is W9.

#### 1.6.9 Selection (FC-060, FC-061; D-175, D-176)

`hikasa_folder::select` is the one owner of the script `select` vocabulary (T11): the CLI's `--select`, W8's `select`
and W9's Edit menu call it.

```rust
// hikasa_folder::select
bitflags! { #[derive(Serialize, Deserialize)] pub struct SideSet: u8 { LEFT = 1; RIGHT = 2; } }
pub enum SelSide { Left, Right, All }
pub enum SelState { All, Exact, Diff, Newer, Older, Orphan }
pub enum SelKind { All, Files, Folders }
pub enum SelectMask { Result { side: SelSide, state: SelState, kind: SelKind }, EmptyFolders }
impl SelectMask { pub fn parse(word: &str) -> Result<SelectMask, FolderError>; }     // Display gives the canonical word
/// What `select` sees. `expanded(node)`: `None` = every folder is open (a headless `expand all`).
pub struct SelectScope<'a> { pub display: &'a DisplayOptions, pub expanded: Option<&'a dyn Fn(NodeId) -> bool> }
#[derive(Debug, Clone, Default, PartialEq, Eq)] pub struct Selection { /* BTreeMap<NodeId, SideSet> */ }
impl Selection {
    pub fn insert(&mut self, node: NodeId, sides: SideSet); pub fn remove(&mut self, node: NodeId, sides: SideSet);
    pub fn get(&self, node: NodeId) -> SideSet; pub fn is_empty(&self) -> bool; pub fn len(&self) -> usize;
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, SideSet)> + '_;
    pub fn restrict(&mut self, side: Side);                                          // Shift+Left/Right
    pub fn invert(&mut self, read: &TreeRead<'_>, scope: &SelectScope<'_>);          // every shown row, every present side
}
/// The masks are OR-ed (SCR-002). Adds to `into`, which starts empty for script `select`.
pub fn select(read: &TreeRead<'_>, scope: &SelectScope<'_>, masks: &[SelectMask], iu: bool, into: &mut Selection);
pub fn select_all_files(read: &TreeRead<'_>, scope: &SelectScope<'_>, into: &mut Selection);
/// FC-062: what `Open`, `Compare To` compare. Two selected items anywhere → those two, in tree order; one item with both
/// sides → its pair; one item and a Space `mark` → those two. Otherwise `None`.
pub fn open_pair(read: &TreeRead<'_>, sel: &Selection, mark: Option<(NodeId, Side)>) -> Option<Pair<Spec>>;
```

**Words.** One word per mask, `.`-separated, case-insensitive, `lt` and `rt` for `left` and `right`. The three slots are
in order and optional; a word fills the first slot it fits, so a leading `all` is the side:

| Word | Mask |
|---|---|
| `all`, `all.all.all` | every side, state, kind |
| `left`, `lt` | side left, all, all |
| `left.diff.files` | as written |
| `diff`, `diff.files` | side all |
| `files`, `all.files`, `all.all.files` | side all, state all, files |
| `newer.folders` | side all, newer, folders |
| `empty.folders` | `EmptyFolders` |
| `left.left`, `files.left`, `bogus`, an empty word | `InvalidRules` naming the word |

**Matching, file-like nodes.** With side `s`; `All` tries both sides and selects each that matches. The node must be
present on `s`; the category is `read.category(id, iu)`. Only `all` matches `Unknown`, `Error` and `Filtered`.

| State | Matches when |
|---|---|
| `exact` | category `Same`, both sides present |
| `diff` | both sides present, category not `Same`, and `diffs` minus `TIME` is not empty (or a content result that differs): *differences other than timestamp*, SCR-002 |
| `newer` | `s` is the newer side (`LeftNewer` on left, `RightNewer` on right) |
| `older` | the other side is the newer one |
| `orphan` | `LeftOrphan` on left, `RightOrphan` on right |
| `all` | present |

**Folders** match a result mask with kind `all` or `folders` only if all their contents match (SCR-002), on the same
side: present on `s`, scanned, and every shown row below it matches by these tables, recursively; a folder's own state
plays no part. A folder that failed to list matches state `all` and no other: it is an error row, so `--select all`
brings it to the plan, which shows it as `skip compare_error` and exits 100. An empty folder matches `all`, `orphan` when it is one, `exact` when it is on both sides. `EmptyFolders`:
a folder with no file below it, on each present side. **Scope:** rows are `read.visible_rows(scope.display, …)`. A
collapsed folder is one row: it can match, its children are not selected (*`expand` is required before `select` sees
subfolder contents*), and its match still reads all that is below it, shown or not expanded. The pass is one bottom-up
walk, O(rows).

**Visible contents (D-176).** What a selection means to an operation is one function, used by every plan:

```rust
pub struct Include { pub hidden: bool, pub display: DisplayOptions }
pub struct Resolved { pub node: NodeId, pub sides: SideSet, /** sides on which the node's whole subtree is included */ pub whole: SideSet }
pub fn resolve(read: &TreeRead<'_>, sel: &Selection, include: &Include) -> Vec<Resolved>;      // pre-order
```

1. A folder selected on a side contributes its **visible** contents: the rows `include.display` shows, without items
   hidden by file filters. With `include.hidden` it contributes everything: filtered rows, rows the display filter
   hides, and name-only items (D-45). The same holds below a folder the model has no contents for: rule 6 of §1.6.10
   lists it through the comparison's filters, and what they hide is not touched without `include.hidden`.
2. A folder and any row below it selected on one side: only the rows below count, on that side (docs/04 §8).
3. `whole` holds a side when nothing below the folder on that side is left out: no filtered row, no hidden row, no
   `NotScanned` folder, no error row. A row a filter or the display hides leaves its folder not `whole` even when it is
   selected by itself. Only then may a plan treat the subtree as one item (one `rename`, one `trash`, one `remove_dir`).

**`include_hidden_by_default` flips to `false` (revises OPT-009's default).** docs/04 §8 and plan W6 say operations
honour the filters by default, docs/15 says the option is `true`, and both cannot be. Operating on what the user cannot
see is how data is lost, so the safe reading wins, and the option remains for those who want it. Explicit values in
`options.toml` keep working; every dialog and `--include-hidden` overrides it.

**Find Filename (FC-061).** Navigation has no CLI form; the pure function is the core part, the Find panel is W9:

```rust
pub struct FindQuery { pub text: String, pub case: bool, pub whole_word: bool, pub regex: bool, pub wrap: bool, pub up: bool, pub sides: SideSet }
pub struct Finder { /* Pattern or folded text */ }
impl Finder {
    pub fn new(query: &FindQuery) -> Result<Finder, FolderError>;       // a bad regex: `invalid_input`
    /// The next row after `from` in direction `up`, wrapping with `wrap`; matches the name on `sides`.
    pub fn next(&self, read: &TreeRead<'_>, rows: &[VisibleRow], from: Option<usize>) -> Option<usize>;
}
```

Names match by `hikasa_core::case::fold` substring without `case`, by `hikasa_formats::Pattern` with `regex`.
`whole_word` tests the neighbours of a match against letters, digits and `_`.

#### 1.6.10 Planning operations (FC-070…FC-074, FC-076, FC-077; D-177)

Every folder operation is built by **one** builder (`ops/build.rs`), which reads the model and calls the VFS only as
rule 6 says. `hikasa_folder::ops` plans; `hikasa_vfs::ops` executes.

```rust
// hikasa_folder::ops
pub enum PathStructure { Relative, Base, None }                    // FC-072; script path:(relative|base|none)
pub enum TouchFrom { At(FileTime), OtherSide { to: Side } }        // "now" is resolved by the Service (T1)
pub struct Recycle { pub use_bin: bool, pub fallback: bool }       // §1.6.12
pub enum RenameMask { Name(String), Dos(String), Regex { old: String, new: String } }
pub enum FolderOp {
    Copy { to: Side, overwrite: OverwritePolicy },            // default Always: the operation names the replacement
    Move { to: Side, overwrite: OverwritePolicy },
    Exchange,
    Delete { sides: SideSet, recycle: Recycle },
    Rename { sides: SideSet, mask: RenameMask },
    Touch { sides: SideSet, from: TouchFrom },
    Mkdir { sides: SideSet, parent: NodeId, name: String, folder: bool },   // `folder`: a trailing separator was given
    CopyTo { sides: SideSet, target: Spec, folder: bool, structure: PathStructure, overwrite: OverwritePolicy },  // default Fail
    MoveTo { sides: SideSet, target: Spec, folder: bool, structure: PathStructure, overwrite: OverwritePolicy },  // `folder`: a trailing separator
}
pub struct OpScope { pub selection: Selection, pub include: Include, pub iu: bool, pub read_only: ReadOnlySides }
pub struct ReadOnlySides { pub left: bool, pub right: bool }
pub struct OpSettings { pub preserve: Preserve, pub links: LinkPolicy, pub backup: Option<BackupPolicy> }
pub enum OpVerb { Copy, Move, Exchange, Delete, Rename, Touch, Mkdir, CopyTo, MoveTo, Sync }   // snake_case in JSON
pub struct ItemMeta { pub side: Option<Side>, pub node: Option<NodeId>, pub path: String, pub conflicts: Conflicts,
                      pub answer: Option<bool> }
pub struct OpPlan {
    pub verb: OpVerb,
    pub bases: Pair<Spec>,
    pub plan: hikasa_vfs::ops::Plan,  // `OpKind::Batch`
    pub meta: Vec<ItemMeta>,          // same length and order as `plan.items`
    pub exec: ExecOptions,            // fence, archive backups
    pub warnings: Vec<Warning>,
}
pub fn plan_op(read: &TreeRead<'_>, vfs: &dyn Vfs, scope: &OpScope, op: &FolderOp, settings: &OpSettings, ctl: &Ctl<'_>)
    -> Result<OpPlan, FolderError>;
```

An `OpPlan` is plain data with `Serialize`, `Clone` and no borrow of the model: a GUI dialog holds it across frames. **A
dry run prints this value, and execution takes this value.**

**Builder rules (normative).**

1. **Sources.** `resolve(read, selection, include)`, restricted to the operation's source side. A row selected only on
   the other side is no source. No source at all → `InvalidRules "nothing selected on the <side>"` (`invalid_input`).
2. **Targets are the counterpart's.** A pair writes to the other side's own spec (`TreeRead::spec`), whatever the name
   looks like: `x.txt` ↔ `x.csv`, NFC ↔ NFD, `A.txt` ↔ `a.txt`. An orphan writes to `vfs.child(<target folder>, <raw
   source name>)`, where the target folder is the other side's own folder of the parent row. So nothing joins strings,
   an aligned pair stays aligned, and a name that is not UTF-8 survives.
3. **Missing folders.** Going down from the base, a row absent on the target side gets a `create_dir` (source = its
   folder, so its metadata is applied after its subtree) at the depth it reaches, once. With only `a/b/c.txt` selected
   in an orphan `a/b`, the plan creates `a`, `a/b`, then copies. A folder that exists gets no item: items below it are
   depth 0. In a flat comparison a row's folders are no rows: the plan `lstat`s each folder on the way to an orphan's
   target once and plans the missing ones the same way, from the source's own folder.
4. **Order within a side:** pre-order for copies, moves, touches and renames that do not chain; post-order for deletes;
   `move`'s `remove_dir` of a source folder after its contents, only for folders whose subtree is `whole`. Sync order:
   §1.6.14. What lies in a folder item the plan skips (a refusal, a declined answer) is skipped for the same reason, at
   planning: a plan shows what will not run, and execution leaves nothing `not_run` that the plan let through.
5. **Kinds.** A file row → `copy`; an unfollowed link row → `link` with `LinkPolicy::Copy`, else `skip symlink`; a
   followed row → `copy` with `follow_source`, its children read through the link; `Other` → `skip special_file`. Inside
   an archive source a link row is skipped with `archive_symlink_skipped` (D-142).
6. **Rows the model has no contents for.** A selected folder that is `NotScanned`, or a filtered one with
   `include.hidden` that was never listed (D-45), is listed once, at planning (`ops/unlisted.rs`), in the comparison's
   order and through the comparison's filters (`Store::resolved`): what they hide is not touched without
   `include.hidden`. From then on its entries are what rows are: rule 5's kinds, rules 8 and 9 against what an `lstat`
   finds, a target the filters hide kept as `skip filtered`, stamps from the listing (rule 10), conflicts by §1.6.11. A
   folder goes as one `rename` (rule 7), `trash` or `remove_dir` only when nothing below it stays (a filtered entry, an
   entry the listing could not read); otherwise item by item, and a folder that keeps something stays. `structure none`
   places each file by its name. The first item keeps the row's `node`, the others name no row and carry their path.
   That and the checks of rule 8 are the only directory I/O of a plan; name-only entries are `stat`'d once for their
   stamp. `Ctl` as for W3's planning: `Err(Cancelled)` before anything is written. `touch` and `exchange` take no rule 6:
   a touch of such a folder sets its own time, an exchange of a pair of them is `skip compare_error`.
7. **One item for a whole subtree.** `Move`, `MoveTo`: a folder `whole` on the source side whose target is absent
   becomes one `rename`, under D-97's conditions (same back-end, `RENAME` on both sides, device ids not known to differ:
   one `stat` per such root). At execution `CrossDevice` expands in place (`Outcome::Expand`), as W3. `Delete` with the
   bin: §1.6.12. Without `whole`, row by row: hidden items stay, and so does their folder. A folder that would move onto
   or into itself (two bases that are one folder, by any spelling) is an item error at planning, as a file is
   (`check_nesting`). An opened archive that goes `whole` is the file it is: with no target, and in a move also onto its
   pair or what stands at its name, which it replaces as a file does (§1.6.25 W6b note 10).
8. **Conflict with a row of another kind.** Different kinds never align (D-20), so a file may meet a folder of its name
   in a sibling row. The builder looks up the name among the parent's children on the target side: a folder target →
   item error `is_a_directory`, whatever the policy (D-96). A file or link target for a folder source → item error
   `already_exists` unless the policy is `Always`, then `create_dir` with `replaces`, its backup, and the conflicts a file
   that replaced it would have (§1.6.11): the human line says `replace with folder`. A target a filter hides is `skip
   filtered` here too.
9. **Overwrite policy** as D-96, decided from the model: `Fail` → item error `already_exists`; `Skip` → `skip exists`;
   `Newer` → the row's `TimeRelation` with the rules' tolerance, `skip not_newer` otherwise (unknown → not newer);
   `Always` → `replaces`. A replaced target gets `conflicts` by §1.6.11.
10. **Stamps.** Every item made from a row carries `expect`: `source` and `target` from `EntryView`; `create_dir` and
    `remove_dir` carry the kind only. An item of rule 6 carries what the listing and its `lstat` saw, for a delete, a move
    and a replace alike. A followed source is compared through the link (`follow_source`), a followed
    target stamps only `kind: Symlink`, since a replace acts on the link. A name-only side (D-45) is stat'd by rule 6.
11. **Capabilities.** `vfs.capabilities(spec)`, memoized per base and per archive root: a row whose side lacks the bit
    it needs (`WRITE`, `REMOVE`, `RENAME`, `MKDIR`, `SET_TIMES`) is `skip capability` and one `capability_missing`
    warning names the side, so a single-stream archive is read-only with no case of its own (§1.5.17).
12. **Fence rule 1 and 3** (§1.6.4) are applied here as `skip followed_link`, `skip compare_error`.
13. **Move** = `Action::Move`, Exchange = `Action::Exchange` on a pair and `Action::Move` on an orphan, in pre-order
    over folder pairs.
14. **Touch.** `At`: every resolved row with `Action::Touch`, `times.modified`; folders too, *where the OS allows*: a
    failure is that item's. `OtherSide`: rows with both sides; `to` is the side touched; its time is the other side's
    `mtime` from the model; a row with one side gives no item. `created` stays `None`.
15. **Mkdir (FC-077).** One name per chosen side under `parent`'s own folder on that side; a parent missing on a side
    gets rule 3's chain. `ops::plan_mkdir`'s D-150 rule decides between `create_dir` and `create_archive` (`folder` =
    the trailing separator). Name checks: §1.6.13's.
16. **CopyTo, MoveTo (FC-072).** The target is resolved with `SpecUse::Folder`; missing: its chain is planned at depth
    0, ending in `create_archive` when its final name has a writable mask (D-150), as `plan_mkdir` does. A path:

| `structure` | `a/b/x.txt` with the selection's common folder `a` | Note |
|---|---|---|
| `Base` | `<target>/a/b/x.txt` | |
| `Relative` | `<target>/b/x.txt` | *minimum common structure*: the longest folder prefix common to all sources of that side, in whole components; rows of two sides use their own |
| `None` | `<target>/x.txt` | flat |

   Folders needed below the target get rule 3's items; a file in the way is rule 8. Two sources with one target path
   (flat names, or `sides` both): the first in plan order goes, left before right, and the others are `skip
   duplicate_target`; a file and a folder of one path count as two sources, while two folders merge into the one the plan
   makes, and what a folder renamed whole takes along counts as gone there. `check_nesting`, `check_holder` and
   `check_move_inside` (W3, W5) run for each root, so a target inside a selected source, and an archive copied into itself,
   are item errors. A folder planned only as the way to a source is dropped when nothing goes there: a refused `moveto`
   leaves no folder behind. `folder` (a trailing separator on `--target`) makes a missing target a folder whatever its name
   looks like, as `mkdir`'s.
17. **Archives.** A plan into, out of or within an archive needs nothing more: `Spec`s are members, `begin`, two-phase
    moves and rebuilds happen in `execute_with`. `archive_backups` of §1.6.5.

#### 1.6.11 Conflicts and confirmation (D-178)

Confirmation is data (plan: *prompt / yes-to-all / no-to-all as data*), so the CLI, the dialogs and W8's `option
confirm:` are three drivers of one machine.

```rust
// hikasa_folder::ops::conflict
bitflags! { #[derive(Serialize, Deserialize)] pub struct Conflicts: u8 {
    READ_ONLY = 1; SYSTEM = 2; OVERWRITE_NEWER = 4; REPLACE_DURING_MOVE = 8; NO_RECYCLE_BIN = 16; OVERWRITE_BACKUP = 32; } }
// hikasa_folder::ops::confirm
pub enum QuestionKind { Operation, ReadOnly, System, OverwriteNewer, ReplaceDuringMove, NoRecycleBin, OverwriteBackup }
pub struct Declined;                                     // Cancel, or No to the operation: nothing runs
pub struct Question { pub kind: QuestionKind, pub item: Option<u32>, pub remaining: u64, pub text: String }
pub enum Reply { Yes, No, YesToAll, NoToAll, Cancel }
pub enum ConfirmMode { Prompt, YesToAll, NoToAll }
/// `[file_operations]` as data; `hikasa_session` fills it (`ConfirmPolicy::from_options`).
#[derive(Debug, Clone, Copy)] pub struct ConfirmPolicy { pub copy: bool, pub move_: bool, pub delete: bool,
    pub read_only: bool, pub system: bool, pub overwrite_newer: bool, pub replace_during_move: bool }
pub struct Confirmation { /* cursor over the questions */ }
impl Confirmation {
    pub fn new(plan: &OpPlan, policy: &ConfirmPolicy, mode: ConfirmMode) -> Confirmation;
    pub fn next(&self) -> Option<&Question>;               // `None`: nothing left to ask
    pub fn reply(&mut self, reply: Reply);
    /// The plan with its answers: declined items become `Action::Skip`, reason `declined`; `clear_read_only` as answered.
    /// `Err(Declined)` after `Cancel` or a `No` to `Operation`.
    pub fn finish(self, plan: OpPlan) -> Result<OpPlan, Declined>;
}
```

| Conflict | Set when an item… | Policy key (`[file_operations]`) | Question |
|---|---|---|---|
| `READ_ONLY` | replaces or removes a target with DOS `R`, or a Unix mode without any write bit (`mode & 0o222 == 0`; owner is not resolved, T1); changes a member of an archive whose outermost file is so (each such item, one `stat` per file); one `trash` of a folder with such an entry below it, as the model or rule 6's listing knows it | `confirm_read_only` | Replace or delete read-only `<path>`? |
| `SYSTEM` | replaces or removes a target with DOS `S` | `confirm_system` | Replace or delete system file `<path>`? |
| `OVERWRITE_NEWER` | replaces a target strictly newer than its source, by the row's `TimeRelation` | `confirm_overwrite_newer` | Overwrite newer `<path>`? |
| `REPLACE_DURING_MOVE` | a `move` with `replaces` | `confirm_replace_during_move` | Replace `<path>` with the moved file? |
| `NO_RECYCLE_BIN` | a removal with `use_bin` whose spec lacks `TRASH` (§1.6.12) | always asked in `Prompt` | `<path>` cannot go to the recycle bin. Delete it permanently? |
| `OVERWRITE_BACKUP` | its backup (§1.6.5) takes the name of a file this plan did not make; for an archive's backup, each item that rewrites it | `confirm_overwrite_newer` | The backup of `<path>` would replace a file that stands there. Replace it? |

Conflicts come from the row's `EntryView`; a key that is `false` drops that question: the item proceeds. A `create_dir`
that replaces a file has the conflicts of a file that would replace it. **Operation
question:** one, first: `confirm_copy` for copy and copyto; `confirm_move` for move, exchange, moveto; `confirm_delete`
for delete; none for rename, touch, mkdir, where each item is named; sync: always (*confirmation shows counts*, FS-005),
unless the mode is not `Prompt`. Its text carries items and bytes per action.

**Modes and replies.** Items are asked in plan order, then class by class per item (read-only, system, newer, replace,
bin). `YesToAll` and `NoToAll` as a **reply** answer the remaining questions of that kind: yes to read-only is not yes
to system. As a **mode**: `YesToAll` answers everything yes; `NoToAll` answers `Operation` yes and every item question
no, so the items without a conflict proceed. A declined item blocks its parents' `remove_dir` (`children_failed`,
D-103), and what lies in a declined folder, from any source, is declined with it. `Cancel` writes nothing. FC-078's *Prompt / Yes to All / No to All* is the mode, with `sync_confirmations`
(OPT-009) as its default in sync plans.

| Adapter | Mode | With a question left in `Prompt` |
|---|---|---|
| CLI, `--yes` or `--confirm yes-to-all` | `YesToAll` | none |
| CLI, `--confirm no-to-all` | `NoToAll` | none |
| CLI, default, a terminal on stdin | `Prompt` | one line per question on stderr, `[y]es [n]o [a]ll [s]kip all [c]ancel`, read by `cli/hikasa-cli/src/confirm.rs` on a thread of its own; Ctrl-C ends the prompt at once as `Cancel`, exit 100 |
| CLI, default, no terminal | `Prompt` | exit 100 `invalid_input`, nothing touched: "`<n>` questions need an answer: pass --yes or --confirm no-to-all, or --dry-run to see the plan", with the kinds and counts |
| CLI `--dry-run` | none | the plan and each item's conflicts; no answers |
| GUI | `Prompt` | dialogs, §1.6.20; W9's list shows conflicts |
| Scripts (W8) | `option confirm:` | `silent` + `Prompt`: "Attempted to open a dialog when running silently" (SCR-003) |

**Rule of D-77 stands.** In the CLI `delete`, and a sync that deletes or replaces, ask their `Operation` question
whatever `confirm_delete` says, so a script's safety never depends on a settings file.

#### 1.6.12 Delete and the recycle bin (FC-073; D-179)

`Delete { sides, recycle }` removes on each chosen side, post-order, from the model. `Recycle::use_bin` = *Use recycle
bin if possible*, `fallback` = *may fall back*.

| Row | `use_bin = false` | `use_bin = true` |
|---|---|---|
| file, link, special file | `remove` | `trash` |
| folder, `whole` | `remove` of its contents, `remove_dir`: post-order from the model | **one** `trash` of the root (D-98), where capability allows; it asks for what lies below it (§1.6.11), and right before it acts, checks that what lies below is what the plan saw (`verify_below`) |
| folder, not `whole` | its visible contents row by row; the folder stays | same with `trash` per item |
| spec without `TRASH` (a member, a blob) | `remove` | `remove` with conflict `NO_RECYCLE_BIN`: *a member is deleted for good only after a rebuild* (§1.5.17) |

W3's primitive never falls back, and neither does the plan unless told to: a `trash` that fails at execution (no bin on
that mount, a share) fails the item with the file in place. With `trash_fallback` it becomes `Outcome::Expand` into
permanent items, in place, with one `trash_fallback` warning naming the count, so the report shows what happened. Those
items are only what the plan saw, as it saw it: a file by its own stamp, a folder by the items stamped at planning
(`PlanItem::fallback`, post-order). Nothing lists at execution: a file that came since is no item and stays, and its
folder's `remove_dir` fails with it; a changed file fails as its stamp says; a `trash` with no such items fails. **A whole
folder's `trash`** (W6c review) takes everything below it, so the planner always stamps that subtree in `fallback`, with
or without the fallback, and sets `PlanItem::verify_below`: right before the `trash`, execution walks the folder (links
not entered) and requires the same names, the same stamps and nothing added; any difference fails the item "changed since
the plan was made", naming what differs, and nothing moves. An entry the comparison never saw (a name filter's) is such a
difference: that folder is not trashed whole, as its `remove_dir` would fail without the bin. `fallback` is the CLI's `--recycle-fallback`, a dialog's check box *If the recycle bin
fails, delete permanently*, and off by default: *if possible* in the docs is answered at planning for what is known, and
never silently at run time. `clear_read_only` applies to `remove`, not `trash`. FS-005's *Delete to recycle bin if
possible* is this table.

*Require explicit side selection* (`require_side_selection_in_delete`) is the GUI's; the CLI has `--side` as a required
flag. `Delete` removes a side's item and leaves the other side's: the row becomes an orphan on refresh.

#### 1.6.13 Rename (FC-074; D-180)

```rust
// hikasa_folder::ops::rename (pure; tables below, §6.6 A6.18)
pub fn dos_rename(name: &str, mask: &str) -> String;
/// FC-074's *computed base mask*: per part, `?` where the names differ, `*` where their lengths differ.
pub fn base_mask(names: &[&str]) -> String;
pub struct RegexRename { /* compiled */ }
impl RegexRename {
    pub fn new(old: &str, new: &str) -> Result<RegexRename, FolderError>;
    pub fn apply(&self, name: &str) -> Result<Option<String>, FolderError>;   // `None`: no match
}
// hikasa_formats::regex_engine (additive; D-180)
impl Pattern {
    pub fn group_names(&self) -> Vec<Option<String>>;
    /// Byte ranges of the groups of the first match at or after `start`; group 0 is the match.
    pub fn captures_at(&self, haystack: &str, start: usize) -> Result<Option<Vec<Option<Range<usize>>>>, PatternError>;
}
```

**DOS masks (normative).** Split a name at its last `.` by D-07 (`stem`, `ext`; `.bashrc` and `a.` have none). A mask
**with** a `.` is split at its last `.`; the result is `expand(stem, mask stem)`, then `.` + `expand(ext, mask ext)`
when that is not empty. A mask **without** one is `expand(whole name, mask)`. `expand(part, m)`: position `i = 0`; for
each mask character: `?` copies `part[i]` if there is one, `i += 1`; `*` copies the rest of `part`, `i = len`; any other
character is written, `i += 1`. Characters are Unicode scalar values.

| Name | Mask | Result | | Name | Mask | Result |
|---|---|---|---|---|---|---|
| `abc1.txt` | `abc?.bak` | `abc1.bak` | | `a.txt` | `*.` | `a` |
| `abcd.txt` | `x?cd.*` | `xbcd.txt` | | `Makefile` | `*.bak` | `Makefile.bak` |
| `report.txt` | `*_old.*` | `report_old.txt` | | `Makefile` | `*.*` | `Makefile` |
| `a.tar.gz` | `*.zip` | `a.tar.zip` | | `.bashrc` | `*.bak` | `.bashrc.bak` |
| `a.txt` | `b` | `b` | | `abc.txt` | `?` | `a` |
| `a.txt` | `b*` | `b.txt` | | `a.txt` | `??.t?` | `a.tx` |
| `longname.txt` | `?????.*` | `longn.txt` | | `é.txt` | `?x.*` | `éx.txt` |

`base_mask`: stems and extensions separately. Per position: all equal → that character, else `?`; lengths differ → `*`
after the common length. Extensions are joined with `.` when any name has one. `["abc1.txt", "abc2.txt"]` → `abc?.txt`;
`["a.txt", "bb.txt"]` → `?*.txt`; one name → itself.

**Regular expressions.** `old` matches the **whole** name: `Pattern::compile("^(?:<old>)$", false,
&RegexLimits::default())`, so `regex` first and `fancy-regex` only where backreferences or look-around ask for it, with
the limits of user patterns (T9). `(?i)` is written in the pattern. `new`: `$1`, `${1}`, `${name}`, `$$`; anything else
after a `$`, a group that does not exist → `invalid_input` at planning. The expansion is in-house, the same over both
engines (the crates' own treat `$1a` differently). A backtrack limit at run time fails that item. A name without a
match: `skip no_match`.

**`hikasa_formats::Pattern` is reused** rather than `regex` being added to `hikasa-folder`: one engine policy (T11),
which is why `hikasa-folder` gains `hikasa-formats` (§1.6.21) and not two crates. `fancy-regex` itself is approved
(`docs/02`, M1) and in the workspace; plan §3 lists only M2's new crates.

**Plan.** Each resolved row on each chosen side gets one `rename` of the row itself, never its contents (a selected
folder is one item; rule 2 of §1.6.9 still holds). `RenameMask::Name` needs one row. The new name must not be empty,
hold a `/`, or be `.` or `..`, and `vfs.child(parent, name)` must accept it, which on Windows applies
`windows_name_problem` (§1.5.7) and refuses `\`: otherwise an item error `invalid_input`. A name unchanged: `skip
same_name`. A name that is not valid UTF-8: item error (nothing can be matched in it). **Collisions**, per folder and
side:

| Case | Result |
|---|---|
| Two items reach one name (folded when the back-end is case-insensitive) | the first in plan order goes on; the others fail `already_exists` at planning. A folder is a real one: in a flat comparison rows of two folders never collide |
| An existing item that is not being renamed holds the name | item fails `already_exists`: **a rename never replaces** |
| A chain, `a → b`, `b → c` | ordered: the free end first (`b → c`, then `a → b`) |
| A cycle, `a → b`, `b → a` | broken by a hidden name: `a → .<a>.hikasa-ren[-n]`, `b → a`, `.<a>… → b` |
| Case-only on a case-insensitive back-end | one `rename` (W3, `is_respelling`) |

#### 1.6.14 Sync engine (FC-078, FS-001, FS-002, FS-004, FS-005; D-181, D-182)

```rust
// hikasa_folder::sync
pub enum SyncPreset { UpdateLeft, UpdateRight, UpdateBoth, MirrorLeft, MirrorRight, Custom }   // default UpdateRight
pub enum SyncAction { Leave, CopyRight, CopyLeft, DeleteLeft, DeleteRight }  // leave, copy_right, copy_left, delete_left, delete_right
pub enum Situation { NewerLeft, NewerRight, AddedLeft, AddedRight, DeletedLeft, DeletedRight, Different, Same }
pub struct SyncMatrix { pub newer_left: SyncAction, pub newer_right: SyncAction, pub added_left: SyncAction, pub added_right: SyncAction,
                        pub deleted_left: SyncAction, pub deleted_right: SyncAction, pub different: SyncAction, pub same: SyncAction }
impl SyncMatrix { pub fn of(preset: SyncPreset) -> Option<SyncMatrix>; pub fn parse(text: &str, base: SyncMatrix) -> Result<SyncMatrix, FolderError>;
                  pub fn get(&self, s: Situation) -> SyncAction; pub fn validate(&self) -> Result<(), FolderError>; }
pub enum OrphanReading { #[default] Added, Deleted }
pub struct SyncOptions { pub preset: SyncPreset, pub matrix: SyncMatrix /* for Custom */, pub orphans: OrphanReading,
    pub create_empty: bool, pub within_archives: bool, pub recycle: bool, pub recycle_fallback: bool,
    pub overrides: BTreeMap<String, SyncAction> }                       // FS-004, base-relative path; a folder covers its subtree
pub struct SyncRules { #[serde(flatten)] pub folder: FolderRules, pub sync: SyncOptions }   // `[rules]` of a folder-sync session
pub struct SyncRequest<'a> { pub options: &'a SyncOptions, pub just_selection: Option<&'a Selection>,
    pub visible: Option<DisplayOptions>, pub include_hidden: bool, pub iu: bool, pub read_only: ReadOnlySides,
    pub allow_empty_source: bool, pub settings: OpSettings }
pub enum LeaveReason { Same, Matrix, Filtered, CompareError, FollowedLink, ReadOnlySide, Capability, OutOfScope,
                       AmbiguousName /* W6c review */ }
pub struct SyncRow { pub node: NodeId, pub situation: Option<Situation>, pub action: SyncAction,
                     pub reason: Option<LeaveReason>, pub overridden: bool, pub conflict: bool }    // ≤ 16 bytes
#[derive(Default)] pub struct SyncCounts { pub copy_right: u64, pub copy_left: u64, pub delete_left: u64, pub delete_right: u64,
                     pub leave: u64, pub conflicts: u64, pub unresolved: u64, pub bytes_right: u64, pub bytes_left: u64,
                     pub ambiguous: u64 /* W6c review */ }
pub struct SyncPlan { pub rows: Vec<SyncRow> /* tree order */, pub op: OpPlan, pub counts: SyncCounts, pub warnings: Vec<Warning> }
impl SyncPlan { /** FS-001's folder summaries: O(1) after the plan. */ pub fn folder(&self, node: NodeId) -> SyncCounts; }
pub fn sync_folder_rules(rules: &FolderRules, options: &SyncOptions) -> Result<FolderRules, FolderError>;
pub fn plan_sync(read: &TreeRead<'_>, vfs: &dyn Vfs, request: &SyncRequest<'_>, ctl: &Ctl<'_>) -> Result<SyncPlan, FolderError>;
```

**Presets (normative).** `→` = `copy_right`, `←` = `copy_left`, `del L/R` = delete left or right, `—` = leave.

| Situation | Item is… | Update Right | Update Left | Update Both | Mirror to Right | Mirror to Left |
|---|---|---|---|---|---|---|
| `newer_left` | both sides, left newer | → | — | → | → | ← |
| `newer_right` | both sides, right newer | — | ← | ← | → | ← |
| `added_left` | left only | → | — | → | → | del L |
| `added_right` | right only | — | ← | ← | del R | ← |
| `deleted_left` | right only | — | — | — | del R | ← |
| `deleted_right` | left only | — | — | — | → | del L |
| `different` | both, no time difference | — | — | — | → | ← |
| `same` | both, same | — | — | — | — | — |

*Update* never removes and never overwrites a newer file; *different* under Update is a **conflict** (FS-004's
Next/Previous Conflict): nothing says which side wins, so it is left, marked `conflict`. A mirror overwrites: that is
what it is.

**Deleted against added (D-181).** Without Sync Memory (FS-003, P1) an item on one side only can be new there or gone
from the other; FS-003 says so itself. FS-002's matrix has both rows, so both stay valid, stored and honoured: `orphans
= "added"` (default) reads a left-only item as `added_left`, a right-only as `added_right`; `orphans = "deleted"`
(`--orphans deleted`) reads them as `deleted_right`, `deleted_left`. The user who knows what happened says so, every
cell of plan W6's 8 × 5 table is reachable, and FS-003 later decides per item.

**Valid cells.** A cell must name an existing side: the left-only situations (`added_left`, `deleted_right`) take
`copy_right`, `delete_left`, `leave`; the right-only ones `copy_left`, `delete_right`, `leave`; the others all five.
`validate` refuses any other cell with `invalid_input`.

**`--custom <matrix>`.** A comma-separated list of `<situation>=<action>`, kebab or snake case:
`newer-left=copy-right,added-right=delete-right`. The base is the session's matrix, else all `leave`; a preset name may
come first as the base: `mirror-right,same=leave`. An unknown word, a repeated situation, an invalid cell → exit 100.

**Effective rules (`sync_folder_rules`).** Facts a plan needs are not left to the view: `auto_scan_subfolders = true`,
`scan_orphan_folders = true`; `flatten` → `invalid_input` ("a sync needs the folder structure"); `archives = Always`
with `within_archives` (FS-005 *Sync within archives*, over `SpecUse::Folder` bases, D-151), else `AsFiles`, so an
archive is a file to copy. An archive base is a folder either way.

**Row algorithm (normative).** Tree order, every file-like row; folders after their children:

1. Scope: `just_selection` (FC-078 *Just selection*; `resolve`) and `visible` (script `sync visible`) keep a row, else
   `Leave`/`OutOfScope`. `sync` otherwise ignores the selection (SCR-002).
2. Category `Unknown`, `Error`, or below a failed or unscanned folder: `Leave`/`CompareError`, `unresolved += 1`. Below
   such a folder nothing is an orphan, so nothing is deleted.
3. A side hidden by a file filter, unless `include_hidden`: `Leave`/`Filtered`. Both sides hidden are not rows, as for
   the display.
4. Situation from `read.category(id, iu)` and `orphans`. An Ignored row is `same`.
5. Action: `overrides` (longest path, §1.6.6's rule), else the matrix. An override impossible for the row (`copy_left`
   on left-only) is dropped with one `switch_ignored` warning per path.
6. Feasibility on the side the action writes: read-only side, `Leave`/`ReadOnlySide` and one `read_only_side` warning
   per side; capability, `Leave`/`Capability`; a followed folder above, `Leave`/`FollowedLink`.

`conflict` = situation `different` with action `leave`, by the matrix, not by an override.

**Folders.** A folder has no row of its own in the matrix; `SyncPlan::folder` adds up its children. An empty folder on
one side reads the matrix as a file would, and its `copy_*` or `delete_*` happens only with `create_empty` (FS-005 *Sync
empty folders*; script `create-empty`); non-empty ones appear through their contents (builder rule 3). After deletes: a
folder all of whose rows are deleted, on a side it alone has, gets `remove_dir` (or is one `trash` root with `recycle`):
a mirror leaves no empty shells. A folder keeping a filtered or left row stays.

**Plan order (D-182).** `delete_left`, `delete_right` (post-order), then `copy_right`, then `copy_left` (pre-order), as
one `OpKind::Batch`, one `execute_with`, so one rebuild and one backup per archive (D-148). Deletes first: a file `x`
replacing a folder `x` needs the name free (D-20 keeps them two rows), and a full disk is the usual failure of a mirror.
Nothing a copy reads is deleted, since a row has one action.

**Empty sources.** A mirror from a side with nothing on it deletes everything on the other: usually a mount that failed.
`plan_sync` returns `FolderError::EmptySource { side }` (`invalid_input`, exit 100) when the plan would delete on one
side and the other base holds no file once the filters are applied (W6c review; folders emptied entirely are shown, not
refused: §1.6.25 *W6c after review* 6), unless `allow_empty_source`; `--yes` does not answer it. **A comparison that
did not finish is refused** (`is_complete() == false` → `InvalidRules "the comparison is not complete"`) for any plan
with deletes. Error rows do not refuse, they are `compare_error`: warning `sync_incomplete` with the count, exit 100.

**Confirmation.** As §1.6.11, verb `sync`. Conflicts per item: `READ_ONLY`, `SYSTEM`, `OVERWRITE_NEWER` (a mirror over a
newer file), `NO_RECYCLE_BIN`. Backups as §1.6.5. Fence with `writes = true` over both bases, so a sync writes nowhere
else.

#### 1.6.15 Sessions and Disable editing (FC-003; D-183, D-184)

```toml
[specs]
read_only_left = false         # FC-003, per side: `--ro` sets both, `--ro1`, `--ro2` one
read_only_right = false
[rules]                        # folder-compare: FolderRules, with the keys of §1.6.6
ignored = ["vendor", "!vendor/keep.txt"]
[rules.format_overrides]
cpp = false
[rules.sync]                   # folder-sync: FolderRules + this table; schema version unchanged
preset = "update_right"        # update_left | update_right | update_both | mirror_left | mirror_right | custom
orphans = "added"
create_empty = false
within_archives = false
recycle = false
recycle_fallback = false
[rules.sync.matrix]            # read with `custom`; eight keys, sparse against all `leave`
newer_left = "copy_right"
[rules.sync.overrides]
"a/b.txt" = "leave"
```

`SessionRules` gains `Sync(SyncRules)`, replacing `Opaque` for `folder-sync` (D-81's *until W6*);
`SessionType::builtin_rules`, `to_value`, `from_value`, `effective_rules` and sparse writes (D-90) treat it as `Folder`.
A folder-sync file written before W6 reads as it is: its table is that shape or empty. `Specs` gains `read_only_left`,
`read_only_right` (`Session::swap_sides` swaps them, `is_dirty` counts them).

**Disable editing is enforced by the Service, not by a view (D-184).** `hikasa_folder::ops::ReadOnlySides` comes from
the session, `--read-only`, or IPC `ReadOnly`, with `--ro` as both. In `folder_op_plan`:

| Case | Result |
|---|---|
| A `folder op` that writes a read-only side (`Delete` with that side, the target of `Copy`, both sides of `Move`, `Exchange`, `Rename`, `Touch`, `Mkdir` on it; sources of `MoveTo`) | `ApiError::ReadOnlySide { side }`, kind `permission_denied`, exit 100, nothing planned |
| `CopyTo` out of a read-only side | allowed: nothing there is written |
| `CopyTo`, `MoveTo` with a `target` at or below a read-only side's base | `ApiError::ReadOnlySide`, as above, before anything is planned. By what the path is (`ops::writes_into`): `..`, a link at the base and links on the way to the target lead where the OS says, a member is the outermost file it lives in, and what cannot be told is refused |
| `folder op ignored`, `exclude` | allowed: they change a session, not a file |
| A sync | row by row: `Leave`/`ReadOnlySide`, warning `read_only_side`; the other side goes on, as a plan is previewed |
| Child sessions | `TextCompareRequest::read_only` is carried to the view; W11 refuses edits with it |

The check is in `Service`, before the builder, so W8's scripts and W9's commands cannot pass it by. A view greys its
commands from the same value (`Ctx`).

#### 1.6.16 Service

```rust
// hikasa_session::api::folder_ops
/// `FolderOp` before resolution: the clock's now, a target as typed, a parent by its path.
pub enum OpRequest { Op(FolderOp), TouchNow { sides: SideSet },
    MkdirIn { sides: SideSet, parent: String, name: String, folder: bool },
    CopyTo { sides: SideSet, target: OsString, structure: PathStructure, overwrite: OverwritePolicy },
    MoveTo { sides: SideSet, target: OsString, structure: PathStructure, overwrite: OverwritePolicy } }
pub struct FolderOpRequest { pub op: OpRequest, pub select: SelectRequest, pub read_only: ReadOnlySides,
                             pub overrides: OpOverrides }
pub struct SelectRequest { pub masks: Vec<String>, pub paths: Vec<(String, SideSet)>, pub selection: Option<Selection>,
                           pub include_hidden: Option<bool>, pub display: Option<DisplayOptions>, pub expand: ExpandWords, pub iu: bool }
pub enum ExpandWords { All, None, Paths(Vec<String>) }
pub struct OpOverrides { pub backup: Option<bool>, pub preserve: Option<Preserve>, pub links: Option<LinkPolicy> }
pub struct FolderOpOutcome { pub report: OpReport, pub meta: Vec<ItemMeta>, pub verb: OpVerb, pub dry_run: bool,
                             pub archive_backups: Vec<(Spec, Spec)>, pub refresh_nodes: Vec<NodeId>, pub warnings: Vec<Warning> }
impl FolderOpOutcome { pub fn ok(&self) -> bool; pub fn exit_code(&self) -> ExitCode; pub fn dto(&self) -> FolderOpDto; }
pub struct FolderSyncRequest { pub options: SyncOptions, pub select: Option<SelectRequest> /* --just-selection */,
                               pub visible: Option<DisplayOptions>, pub include_hidden: Option<bool>, pub iu: bool,
                               pub read_only: ReadOnlySides, pub allow_empty_source: bool, pub overrides: OpOverrides }
pub struct FolderSyncOutcome { pub op: FolderOpOutcome, pub rows: Vec<SyncRow>, pub counts: SyncCounts }
pub enum ContentsKind { Compare(ContentMethod), Crc(SideSet) }
pub struct ContentsRequest { pub select: SelectRequest, pub kind: ContentsKind }
pub struct ContentsDto { /* hikasa.folder_contents.v1 */ }
pub struct RefreshDto { pub report: Option<RefreshReport> /* None: flat, the caller re-runs */, pub warnings: Vec<Warning> }
pub enum OpenTarget { App(usize), Named(String), Associated }
pub struct OpenWithRequest { pub target: OpenTarget, pub items: Vec<(Spec, Spec)> /* item, its base */, pub line: Option<u64> }
impl Service {
    pub fn folder_op_plan(&self, model: &Arc<FolderModel>, req: &FolderOpRequest, job: &JobControl) -> Result<OpPlan, ApiError>;
    pub fn folder_confirmation(&self, plan: &OpPlan, mode: Option<ConfirmMode>) -> Confirmation;   // mode: else options
    pub fn folder_op_dry_run(&self, plan: OpPlan) -> FolderOpOutcome;
    pub fn folder_op_execute(&self, plan: OpPlan, job: &JobControl) -> Result<FolderOpOutcome, ApiError>;
    pub fn folder_refresh(&self, model: &Arc<FolderModel>, nodes: &[NodeId], job: &JobControl, observer: &dyn ScanObserver)
        -> Result<RefreshDto, ApiError>;
    pub fn folder_open_archive(&self, model: &Arc<FolderModel>, node: NodeId, job: &JobControl, observer: &dyn ScanObserver)
        -> Result<RefreshDto, ApiError>;
    pub fn folder_contents(&self, model: &Arc<FolderModel>, req: &ContentsRequest, job: &JobControl) -> Result<ContentsDto, ApiError>;
    pub fn folder_sync_prepare(&self, req: &FolderCompareRequest, sync: &SyncOptions, job: &JobControl) -> Result<PreparedFolderCompare, ApiError>;
    pub fn folder_sync_plan(&self, model: &Arc<FolderModel>, req: &FolderSyncRequest, job: &JobControl) -> Result<SyncPlan, ApiError>;
    pub fn folder_sync_execute(&self, plan: SyncPlan, job: &JobControl) -> Result<FolderSyncOutcome, ApiError>;
    pub fn folder_set_ignored(&self, model: &Arc<FolderModel>, nodes: &[NodeId], on: bool) -> Vec<String>;
    pub fn open_with_commands(&self, req: &OpenWithRequest, job: &JobControl) -> Result<Vec<OpenCommand>, ApiError>;  // §1.6.18
    pub fn open_with_run(&self, commands: &[OpenCommand], wait_for_previous: bool, job: &JobControl) -> Result<(), ApiError>;
    pub fn open_associated(&self, spec: &Spec, job: &JobControl) -> Result<(), ApiError>;
}
```

Each runs through `run_guarded`; `folder_op_execute` calls `execute_with` on the calling thread, with `vfs =
&*self.router()` as both `src` and `dst`, so `same_instance` holds, `copy_file_fast` works and an archive on both sides
is one batch. `FolderError` gains `RefreshBusy`, `RefreshFlat`, `EmptySource { side }`, all `invalid_input` through
`ApiError::from_folder`. Settings: backups from `[backups]`, unless `overrides`; `preserve.created` from the new
`[file_operations] preserve_created` (bool, false, OPT-009; FC-070's *creation date (option)*). The CLI's `folder op`,
W8's interpreter and W9's commands all reach these calls: nothing else plans (T11). `hikasa_core::JobKind` gains
`FolderOp` (`folder_op`), `FolderSync` (`folder_sync`), `FolderRefresh` (`folder_refresh`). `Phase::Scanning` while
planning a tree (rule 6), `Phase::Writing` while executing, `Phase::ComparingContent`, `Phase::Hashing` for §1.6.8.
`ApiOp::FolderOp`, `ApiOp::FolderSync`. `WarningCode` gains `TrashFallback` (`trash_fallback`), `ReadOnlySide`
(`read_only_side`), `SyncIncomplete` (`sync_incomplete`). `ApiError::ReadOnlySide { side }`: kind `permission_denied`,
exit 100. Item errors keep their `ErrorKind`.

#### 1.6.17 CLI and JSON (D-185, D-190)

```
hikasa folder op <VERB> [<LEFT> <RIGHT>] [--session <REF>] <rule flags of folder compare> [SELECT] [CONFIRM] [verb flags]
hikasa folder sync [<LEFT> <RIGHT>] [--session <REF>] (--update-left|--update-right|--update-both|--mirror-left|--mirror-right|--custom <MATRIX>)
    [--orphans added|deleted] [--create-empty] [--within-archives] [--recycle [--recycle-fallback]] [--override <PATH>=<ACTION>…]
    [--just-selection SELECT] [--visible] [--include-hidden] [--allow-empty-source] [--backup|--no-backup] [--read-only left|right|both]
    [--all-rows] <rule flags> [CONFIRM] [--gui [--wait] [--no-launch]]
SELECT  = [--select <MASK>…] [--path <REL>…] [--include-hidden] [--no-expand] [--expand <REL>…] [--display <FILTER>] [--ignore-unimportant]
CONFIRM = [--yes | --confirm prompt|yes-to-all|no-to-all] [--dry-run] [--read-only left|right|both]
```

| Verb | Flags | Plan |
|---|---|---|
| `copy` | `--to left\|right` (required), `--overwrite`, `--no-preserve`, `--preserve-created`, `--symlinks copy\|skip`, `--backup`, `--no-backup` | FC-070 |
| `move` | as `copy` | FC-071 |
| `exchange` | `--backup` refused | FC-071 |
| `delete` | `--side left\|right\|both` (required), `--recycle`, `--recycle-fallback` | FC-073 |
| `rename` | `--side`, one of `--name <NEW>`, `--mask <MASK>`, `--regex <OLD> <NEW>` | FC-074 |
| `touch` | `--side`; `--time <RFC 3339>` \| `--now` (default) \| `--from-other` | FC-076 |
| `mkdir` | `--side`, `--in <REL>` (default the base), `--name <NAME>` | FC-077 |
| `copyto`, `moveto` | `--side`, `--target <SPEC>` (required), `--structure relative\|base\|none` (default `relative`), `--overwrite` | FC-072 |
| `compare` | `--method crc\|binary\|rules` | FC-029(b) |
| `crc` | `--side` | FC-040 |
| `exclude` | `--what items\|type`, needs `--session` unless `--dry-run` | FC-052 |
| `ignored` | `--off`, needs `--session` unless `--dry-run` | FC-033 |
| `open-with` | `--path <REL>`, `--app <NAME\|INDEX>` \| `--associated`, `--side`, `--dry-run` | FC-062 |

- `<LEFT> <RIGHT>` or `--session <REF>`: a session gives specs, effective rules (D-82), read-only and Ignored marks;
  flags apply on top, as for `session run`. Both forms run `folder_prepare_in` + `folder_run`, then plan, with
  `JobKind::FolderCompare` first.
- **Default scope.** No `--select`, no `--path`: **nothing is selected** and a plan-needing verb exits 100 "nothing
  selected: pass --select all, or a mask": a stray command must not act on the whole tree. `--display` defaults to `all`
  (not `differences`), `--no-expand` is off: a headless command sees the whole tree, as a script after `expand all`.
- `--path <REL>`: one row by its path; prefix `left:`, `right:` for one side, else the verb's side. Unknown, or a row
  that is not on that side (W6c review: it selected nothing, exit 0) → exit 100 `not_found`, nothing runs. It never names the base: an empty path, and any spelling that comes to it (`.`, `/`, `left:`, `./`) is
  `invalid_input`, on every verb that takes `--path`; the whole tree takes `--select`.
- `--yes` and `--dry-run` conflict (clap). `--to`, `--side`, `--method` parse through `api::vocab` (T11).
- `folder sync --update-right` = Update Right = script `update:left->right`; `--mirror-left` = Mirror to Left =
  `mirror:right->left`; `update:all` = `--update-both`. Exactly one is required, unless `--session` holds a preset.
- `--just-selection` with `SELECT`; without it `--select` is refused: `sync` ignores the selection (SCR-002).
  `--all-rows` adds `same` rows and folders to the output.
- `session run <REF>` of a `folder-sync` session: `folder sync` with the session's options, `--dry-run`, `--yes`,
  `--confirm`, `--allow-empty-source`; without a terminal and without `--yes` it exits 100, as `folder sync`.
- **`--sync` and `--gui` (CLI-003, CLI-G03; D-190).** `--sync` is now parsed: `OpenArgs::kind = FolderSync` (with `--fv`
  the last wins), `--nobackups` sets `OpenOptions::no_backups` (additive, `#[serde(default)]`, D-46). `folder sync
  --gui` builds `GuiOpenRequest::for_sync` and sends `OpenView { kind: FolderSync, … }`, with preset, matrix and flags
  in `OpenOptions::sync: Option<SyncWords>`. `open_view_request` gains `ViewRequest::Sync`. **The view is W9:** until
  then the GUI answers `Error{unsupported}`, "this window cannot show a Folder Sync view yet", exit 100, as for a
  session type it cannot show. The switch, the IPC and the checks are W6's; what opens is W9's.

**Exit codes** (ARCH-101, M1 §2.21; `ExitCode` needs no new row):

| Outcome | Exit | Envelope |
|---|---|---|
| every item done or skipped; dry run with no item error | 0 | `ok: true` |
| a failed item, an item `not_run` (also without a cancel), cancelled, a `compare_error` row (also in a dry run), a failed rebuild | 100 | `ok: true`, `error: null`, the failures in `items` (as `hikasa.vfs_op.v1`) |
| a base that cannot be loaded | 107 | failure envelope |
| a bad word, nothing selected, unanswered questions, `ReadOnlySide`, `EmptySource` | 100 | failure envelope, nothing written |
| a comparison cancelled before the plan | 100 | failure envelope |

**Human output.** One line per item in plan order, as §1.3.7 with the same words and `escape_controls`, prefixed by the
side's letter where the item has one (`L `, `R `, `- ` for a target outside the bases); new words `exchange <left> <->
<right>`, `skip (<reason>) <target>` with the new reasons; `replace with folder <target>` and `replace with link …`
where a folder or a link replaces a file; ` (backup <spec>)` after a replacing line; ` [read_only,overwrite_newer]` on a
dry run's conflicts; `, <n> not run` at the end of a summary that has such items. Summary as §1.3.7 with verbs `copied`, `moved`, `exchanged`,
`deleted`, `renamed`, `touched`, `created`, and a dry run's `would copy …`. Sync adds one line per non-`same` row:
`<marker> <action> <path>` with `L>R`, `R>L`, `delL`, `delR`, `----`, `<!>` for a conflict, then `# <n> copies right,
<n> copies left, <n> deletes left, <n> deletes right, <n> conflicts, <n> unresolved`.

**JSON.** Appended to `SCHEMAS`, 35 → 38:

| Schema | `result` |
|---|---|
| `hikasa.folder_op.v1` | `{op, dry_run, cancelled, left, right, items: [{action, kind, side: "left"\|"right"\|null, path, source, target, size, replaces, backup, conflicts: [str], answer: bool\|null, status, reason, error}], backups: [{file, backup}], summary: {planned, done, skipped, failed, bytes}, commands: [{program, args, cwd}]\|null}` |
| `hikasa.folder_sync.v1` | `{preset, matrix: {newer_left, newer_right, added_left, added_right, deleted_left, deleted_right, different, same}, orphans, dry_run, cancelled, left, right, rows: [{path, kind, situation, action, overridden, conflict, reason}], items: [as folder_op], counts: {copy_right, copy_left, delete_left, delete_right, leave, conflicts, unresolved, bytes_right, bytes_left, ambiguous}, backups, summary}` |
| `hikasa.folder_contents.v1` | `{op: "compare"\|"crc", method, cancelled, items: [{path, side, result, crc32, error}], counts: {binary_same, rules_same, unimportant_only, important_differences, binary_different, error}}` |

`exclude` and `ignored` print `hikasa.session_write.v1`, or with `--dry-run` `{masks}`/`{ignored}` in it. `rows` hold
`leave` rows with a reason other than `same`, unless `--all-rows`. `action` gains `exchange` and `reason` the new words
in `hikasa.vfs_op.v1` too: additive, and only `folder` commands produce them. `write_json` (§1.5.13) keeps hostile names
safe.

`FolderCompareArgs`' rule flags move to a `#[command(flatten)] FolderRuleArgs` used by `compare`, `op` and `sync`: one
owner, the same flags, and the new ones: `--attrib-filter <EXPR>`, `--unixtype-filter <EXPR>`, `--no-auto-scan`,
`--expand-on-load`, `--expand-differences`, `--ignored <PATH>…`, `--format-on <ID>…`, `--format-off <ID>…`.

#### 1.6.18 Open With, Compare To, Exclude (FC-052, FC-062; D-186, D-187)

**Open With.** D-76's `expand_command` exists; this adds the items and the launch.

```rust
// hikasa_session::launcher (pure trait; like ClipboardProvider, D-100)
pub struct OpenCommand { pub program: OsString, pub args: Vec<OsString>, pub cwd: PathBuf }
pub trait AppLauncher: Send + Sync {
    /// Starts the program and returns when it has ended (a job thread waits: `refresh_when_finished`, `wait_for_previous`).
    fn run(&self, command: &OpenCommand) -> Result<i32, ApiError>;
    /// FC-062 *Associated Application*.
    fn open_associated(&self, path: &Path) -> Result<(), ApiError>;
}
pub struct NoLauncher;                 // `unsupported`; `ServiceInit::new`'s default
pub struct RecordingLauncher { /* tests: records, answers from a script */ }
// hikasa_session::host (adapter-only, like SystemClipboard)
pub struct SystemLauncher;             // std::process::Command (no shell, stdio null); `open::that_detached`
// hikasa_vfs::archive (§1.5.17)
impl ArchiveVfs { pub fn extract_to_cache(&self, spec: &Spec, cancel: &CancelToken) -> Result<PathBuf, VfsError>; }
```

- **Items.** `Local` specs give their path and base. A member goes through `extract_to_cache`, and its `ExpandItem` is
  that path with its folder as base: `%f`, `%p`, `%n`, `%x` describe the extracted file, `%P` is its name.
  `ExpandContext` is built by `Service::open_with_commands`, `InstanceMode` decides the number of commands: per item, or
  per pair with `%…2` variables. A `%…2` without a second item is `invalid_input` (D-76). The words are OS strings: a
  name that is not Unicode reaches the program unchanged. A relative value of `%n`, `%b`, `%F` or `%P` that begins a word
  and starts with `-` is given as `./-…`, so it reaches a program as a path and not as an option. A row missing on a side
  `--side` names is `not_found`, exit 100, and nothing starts.
- **`extract_to_cache`.** An external program needs a name and an extension, which D-140's numeric names do not have, so
  this one call extends D-140: `s<n>/x<k>/<safe name>`, `k` numeric, a fresh folder per extraction. `<safe name>` is the
  member's final name if it passes rules 1–4 and 17 of §1.5.7 and `windows_name_problem`, else `member` plus its
  extension if that passes, else `member`. The file is written through the spool, read-only (`0400`, DOS `R`). It stays
  until `cleanup`: valid for the session (§1.5.17), counted in `used`, never evicted, and a member that does not fit
  `cache_max` is `ArchiveLimit`. Links, encrypted and sparse members: the errors of `open_read`. Same member, same
  stamp: the same path. An edit by the program is not written back (W10/W11 own editing), and the read-only mode says
  so.
- **Run.** `open-with` prints the commands with `--dry-run`, else runs them through `AppLauncher`, `wait_for_previous`
  one after another, the default all at once, each on a job thread; `--associated` calls `open_associated`. Exit 0, or
  100 when a program cannot start; a program's own exit code is not Hikasa's.
- **`refresh_when_finished`** is the view's: it refreshes the rows' parents when the job ends (W9).
- Tests use `RecordingLauncher`, never a real program. Only `cli/` and `gui/` name `SystemLauncher`.

**View types of Open With, Compare To.** `Open With ▸ Text, Table, …` are `ViewRequest`s over `select::open_pair` with a
forced kind (W9); views that do not exist yet are `planned`. **No new subcommand for Compare To:** `hikasa gui open a b`
is its headless form, and a comparison of two named items is `quick`. D-105 lets the view call the pure `open_pair`.

**Exclude (FC-052).** Pure, in `select.rs`:

```rust
pub enum ExcludeWhat { Items, Type }
/// FC-052: masks for the exclude lists, in `NameFilterLists` form: (`exclude_files`, `exclude_folders`).
pub fn exclude_masks(read: &TreeRead<'_>, sel: &Selection, what: ExcludeWhat) -> (Vec<String>, Vec<String>);
pub fn escape_mask(name: &str) -> String;
```

`Items`: a file → its anchored relative mask (`a\b.txt`, with `.\b.txt` in the base), a folder → `a\b\` in
`exclude_folders`. `Type`: `*.<ext>` per distinct extension of the selected files, D-07's extension; a file without one
is excluded as an item. Names are escaped: `[`, `?`, `*`, `;`, `!` at the start, a leading blank and a leading `-` as
one-character sets (`[[]`, `[?]`, `[ ]`). `escape_mask` round-trips: the mask matches that name and no other (a
property). An entry already in the list is not added. FS-004's *Exclude removes items from the plan* is this: rules
change, the view re-runs. Rules are immutable in a model, so Exclude is the one operation that **re-runs**, not
refreshes.

#### 1.6.19 Log model (FC-100; D-188)

```rust
// hikasa_session::log (pure, additive)
pub fn op_log_entries(outcome: &FolderOpOutcome, time: &jiff::Zoned, source: &str, elapsed: Option<Duration>, filter: &LogFilter) -> Vec<LogEntry>;
pub fn sync_log_entries(outcome: &FolderSyncOutcome, time: &jiff::Zoned, source: &str, elapsed: Option<Duration>, filter: &LogFilter) -> Vec<LogEntry>;
```

One `Summary` entry: `<verb> <done> of <planned> items (<skipped> skipped, <failed> failed), <bytes>[, <seconds> s]`,
the text of §1.6.17's summary (one function, T11). One `Error` entry per failed item: `failed <word> <target>:
<message>`, which D-116 always keeps. With `filter.details`, one `Detail` per done or skipped item, the human line, at
most `LOG_DETAILS_MAX` = 10,000 per operation, then `… and <n> more`. Warnings by `push_warnings`. The CLI has `--log`
for diagnostics and no use for this; W8's `log` command does. Ordering is plan order. `elapsed` comes from the adapter
(T1).

#### 1.6.20 GUI hooks (W6d; D-189)

The hooks §1.4.18 and §1.5.17 name, no more. The window gets what a **one-row selection** can do, and nothing that W9's
lists and selection will replace:

- **Commands.** `CopyRight`, `CopyLeft`, `Delete`, `PermanentDelete` of section `folder-compare` become `implemented`;
  `Delete` gains its documented shortcut `Del` and `PermanentDelete` its caption *Delete Permanently*, both missing from
  the catalogue; `docs/reference/commands.md` is regenerated. New predicates `SideWritable(Side)` and `RowSelected`. All
  other Actions stay `planned`.
- **`op_dialogs.rs`** (no egui; check 7): `PlanConfirmDialog`, a `DialogModel` over `Confirmation`: title and text of
  `Question`, counts of the plan, buttons Yes, No, Yes to All, No to All, Cancel; "Do not ask again" sets the question's
  option key (`Prompt`'s `do_not_ask`, D-112), `sync_confirmations` excepted. `DeleteDialog`: sides (Left, Right, Both;
  none preset with `require_side_selection_in_delete`, OK disabled until one is chosen), *Use recycle bin if possible*,
  *If the recycle bin fails, delete permanently* (hidden for `PermanentDelete`, where the first is off and disabled),
  counts, read-only sides greyed. Both drive `Service` calls off the UI thread (D-105) and are refused while a dialog's
  job runs. Help topics `dialog-confirm`, `dialog-delete`.
- **Flow in `folder_view.rs`.** Command → `Service::spawn` plan job → dialog → `Service::spawn` execute job
  (`JobKind::FolderOp` in the job list, Stop cancels: D-115) → `op_log_entries` into the log book → one notice (`W6
  operation results`, D-117: info when clean, warning with *Show log* when items failed) → `folder_refresh` job with
  `refresh_nodes`, then `rebuild`. The window's paths written by an operation are not acknowledged: an open text view of
  such a file sees the change (D-118), which is true.
- **Ids move.** `renumbered` updates the view's open set and selection. A `FullRefresh` is unchanged. `Store::garbage`
  above half the arena after a refresh: the view runs a full refresh.
- **`Open` on a closed archive** calls `folder_open_archive` instead of the re-run of §1.5.14; `rules.open_archives` in
  the view's session is updated, so it round-trips.
- **FC-012.** `expanded_on_load` opens folders after a run; Expand on a `NotScanned` folder runs a refresh of it.
- **FC-029(c).** `Open` remembers `(child view, node, stamps)`; when the child's comparison ends, `apply_child_result`
  updates the row. Its result is the child's: rules-based.
- **Disable editing.** `Ctx` gets the sides; `ReadOnly` from IPC is honoured.
- `--nobackups` of a view: `OpOverrides::backup = Some(false)` for all its operations.

Everything is tested through the catalogue without a window (§1.4.16). W9 replaces the dialogs' text body by the plan's
lists; the models stay.

#### 1.6.21 Dependencies and architecture test (D-191)

- **`open`** 5.4.5 (MIT; checked with `cargo info` on 2026-11-02): `open = { version = "5.4.5", default-features = false
  }`, used by `hikasa-session`, named only in `host.rs` (plan §3 approved it for FC-062; §0.4's *waits for W7/W13* is
  overtaken, since FC-062's associated application is W6). `cargo deny check` must pass; a new licence stops the part
  and is reported.
- **`hikasa-folder`** manifest += `hikasa-formats` (§1.6.13; no cycle: `hikasa-formats` depends only on `hikasa-core`
  among the crates). `hikasa-session` manifest += `open`. No other crate. `fancy-regex`, `regex` stay in
  `hikasa-formats` and `hikasa-text`.
- **Architecture test.** Manifest table as above. Check 2 gains: the token `open::` only in
  `core/hikasa-session/src/host.rs`; `SystemLauncher` joins the adapter-only names (`cli/`, `gui/` and hikasa-session's
  own tests; refused in every test file of `core/`, `cli/`, `gui/`, as `SystemClipboard`). Check 7's list of files
  without egui gains `op_dialogs.rs`, and the T11 rule: `execute_with` is called only from
  `hikasa_session::api::folder_ops`; `hikasa_vfs::ops::execute` also from `vfs_ops.rs`.
- No `unsafe`. `std::process` in `host.rs` is allowed (the banned tokens name `process::exit` only).

#### 1.6.22 Gate G7 and budgets (D-192)

Measured at the end of W6c on an idle machine, release build, fixtures generated by the test; recorded in `docs/02`;
`bench.yml` runs the benches.

| Item | Measure | Where | Threshold |
|---|---|---|---|
| G7 copy | `folder_op_execute` of a prepared plan copying 10,000 files of 4 KiB in 100 folders, local → local, targets absent, against a plain recursive copy in the same test (`fs::copy` + `File::set_modified`, no `fsync`, no temporary name), median of three each, on the same file system | `core/hikasa-session/tests/perf_g7.rs`, `#[ignore]` | ≤ 1.5 × |
| G7 plan | `plan_sync` of a compared tree of 100,000 files, half differing, Mirror to Right | `core/hikasa-folder/benches/sync_plan_100k.rs` | < 0.5 s |
| Memory | `SyncRow` ≤ 16 bytes; `OpPlan` for 100,000 items < 64 MiB (a test on a counting allocator) | `core/hikasa-folder/tests/ops_memory.rs` | listed |
| Refresh | a refresh of one level of 1,000 rows in a tree of 100,000 lists only that level (`MemoryVfs` journal of `list`) | `refresh.rs` tests | one `list` per side |
| Cancel | a hook cancels at byte n of a streamed copy: at most one buffer more is read (`MemoryVfs` counter), the target is untouched; no clock, name holds `cancel` | `ops_batch.rs` | G4 by construction |

G2 and G4 are unchanged. G7 copy has little room (§1.6.3: 1.45 × in a scratch run), so a failure is reported with the
figures, as a failed gate (`docs/17` §7.1), not worked around by relaxing a guarantee. Copies that replace have no gate:
they `fsync`, on purpose.

#### 1.6.23 Hooks for later waves

- **W8.** Script commands map one to one to the calls of §1.6.16, by the same words (`api::vocab`, `SelectMask::parse`,
  `SyncMatrix::parse`):

| Script | Call |
|---|---|
| `select <mask>…` | `select::select`, with `expanded` from `expand`/`collapse` |
| `copy left->right`, `move …` | `FolderOp::Copy { to: Right, overwrite: Always }`, `Move` |
| `copyto …`, `moveto …` | `CopyTo`, `MoveTo`; `path:relative` = `Relative` |
| `delete recyclebin=yes all` | `Delete { sides: LEFT\|RIGHT, recycle: { use_bin: true, fallback: false } }` |
| `rename regexpr <old> <new>` | `RenameMask::Regex` |
| `touch left->right`, `touch all:now` | `TouchFrom::OtherSide`, `At(clock)` |
| `compare crc` | `folder_contents` |
| `sync create-empty mirror:left->right` | `folder_sync_plan` with `MirrorRight`, `create_empty`; `visible` = `SyncRequest::visible` |
| `filter attrib:…`, `filter unixtype:…` | `AttribFilter::parse`, `UnixTypeFilter::parse` |
| `option confirm:…` | `ConfirmMode` |

  `attrib` of the script is FC-075 (P1) in `docs/04` and P0 vocabulary in `docs/12` and plan W8: W8 decides, W6 gives it
  no call. Its primitive `set_attrs` is there.
- **W9.** Selection by pointer feeds `Selection`; dialogs render `OpPlan` and `SyncPlan` unchanged (`SyncPlan::folder`
  for FS-001's summaries); `OpPlan::meta` carries the rows; `Ignored`, `Exclude`, `Rename`, `Touch`, `NewFolder`,
  `CopyTo`, `MoveTo`, `Move*`, `Update*`, `Mirror*`, `Sync*` become `implemented`; `ConflictNext/Prev` walk
  `SyncRow::conflict`; FC-012's glyphs read `ScanStatus`; the results dialog reads `ContentReport`;
  `[tweaks.file_operations] beep_after_long_operations` (OPT-015) at the end of a job; FC-082's *re-stat of a view* can
  build on `refresh_into`. The view of `--sync` and `folder sync --gui`.
- **W10.** Save takes `BackupPolicy::target` with `rel = None` and `write_backup` (`back_up_before_save`).
- **FS-003 (M3).** Sync Memory replaces `OrphanReading` per row: the matrix does not change.

#### 1.6.24 Documentation and version

`docs/01` §3.2 (the new trait methods, `Durability`); `docs/02` (the `open` line, §1.6.22's figures); `docs/03` (session
`[rules.sync]`, `read_only_*`, `ignored`, `format_overrides`, OPT-009 flip); `docs/04` FC-003, FC-012, FC-029, FC-033,
FC-040, FC-052, FC-060…FC-062, FC-070…FC-074, FC-076…FC-078, FC-100, §8 (visible contents, backups, confirmation), §10
FS-001, FS-002, FS-004, FS-005 (presets table, orphans, order, empty sources), §6.2 (attribute and Unix-type filters;
A5); `docs/10` FF-002; `docs/11` (mutations: durability and what a power cut can lose, batches, the fence, backups; the
extraction folder); `docs/12` §1 (the `folder op` row, `attrib` left out with a pointer, `folder sync`), §2 (`--sync`,
`--nobackups`, CLI-G03), §4 (flags table, human table, schemas, 38 in all, exit codes), `session run`; `docs/14` (the
four commands, dialogs, notices, the log); `docs/15` OPT-008 (`folder` keeps the path), OPT-009
(`include_hidden_by_default` default, `preserve_created`); `docs/16` M2 progress; `docs/reference/commands.md`
regenerated.

**Help** (`cargo xtask help-check` clean): new `folder-op` (verbs, selection, confirmation, backups, recycle bin,
masks), `folder-sync` (presets, matrix, orphans, overrides, empty sources), `dialog-confirm`, `dialog-delete`; updated
`folder-compare` (new flags, Ignored, filters), `index`, `session` (`session run`), `settings`, `vfs` (durability),
`exit-codes`, `json-output`, `view-folder-compare`, `archives` (operations, backups of archives). `hikasa help folder
op` resolves through D-67.

**CHANGELOG.** One W6 section in `## Unreleased`, under Added, Data safety, Changed, Fixed (and Internal), that a user of
0.1.92 reads as one release: each part and each review fix updates it in place rather than adding an entry of its own,
so it never tells of a state a later part changed (§1.6.25 *W6d after review* 9). Changed names the two defaults a user
can meet: `include_hidden_by_default` and that a new copy no longer `fsync`s. **Version.** W6 may ship
as pre-release **0.1.92** after W6d, by the steps of 0.1.91 (workspace version, `CHANGELOG` date, tag, packages, CI of
all three systems first); 0.2.0 stays reserved for M2 complete. No part bumps the version on its own. Windows and macOS
run in the pre-merge CI (§6.6 rules).

#### 1.6.25 W6 notes (implementation)

*To be written by each part*, as §1.5.19: every point where the implementation interpreted or extended the text above,
numbered per part.

**W6a.** Where the implementation interpreted or extended §1.6.2–§1.6.8 and §1.6.18, and why.

1. **Exchange with an archive, the other way round.** §1.6.2 defers step 4 only. That is right when `target` is the member.
   With `source` the member and `target` the plain file, step 3 would replace the file on disk before the rebuild, and a
   failed rebuild would lose it. So the rename **on disk** comes last in either direction: the member's rename runs in the
   overlay, the disk's joins `exec.deferred`. After a cancellation, an exchange whose rebuild went through still runs its
   last rename, as "after step 3 the item runs to its end" asks.
2. **`execute_with` refuses two archives too.** The builder's item error is W6b; a hand-built plan gets `unsupported` from
   `run_exchange`, for the reason §1.6.2 gives.
3. **Clear read-only.** `clear_read_only` forces the stream path, so `R` goes just before `commit` and comes back if the
   commit fails. With a backup `R` travels with the file to the backup, and only a backup on another disk clears the target,
   which then still stands. `exchange` clears both sides on disk before step 3, a read-only left side would otherwise fail
   step 4 on Windows, and gives `R` back on both at any failure, the left side's at step 4 too; a plain side against a member
   clears it just before its rename, after the rebuild; a member has none to clear. `run_rename` clears a replaced target,
   or goes by a backup as a copy does. `link` and `create_dir` move the item aside with its `R`, and clear it only as that
   sibling is removed, so a failure has nothing to give back and a backup keeps it; without `RENAME` the item is cleared just
   before it is removed. A hidden copy of an exchange that took `R` from its side is cleared before it is removed. `remove`
   clears and gives back; `trash` does not.
4. **A stamp reads once.** `Seen` carries the two `lstat`s into the action. `run_copy` also keeps the source entry for the
   metadata step, so a copy reads the source once, where W3 read it twice; the metadata is what was seen before the
   content. A source changed in between gets its copy with the older time, which a later comparison shows.
5. **The fence runs before `Roots::touch`,** so a refused item opens no archive. It is step 2 all the same: nothing between
   them looks at the item. Keys of the bases are built once per execution, as an item would otherwise fold each base.
6. **`within` of an archive file.** `Spec::parent` of an archive's root skips the file, so a base given as the file
   (`/a/b.zip`, not `/a/b.zip/`) would never hold its members. `within` compares that file too. The root of an archive is
   not below its own file: it is the same thing.
7. **The fence and a base as a write target.** "Every target" is literal: with `writes`, a `keep_dir` or `touch` of a base
   itself is refused. W6b's builder must not plan one.
8. **`copy_new_file`'s temporary file is not locked.** `create_temp`'s `try_lock` and `still_named` cost a tenth of the
   path, and exist so that a rebuild's `reap` can tell a live writer: `reap` looks only beside an existing target, and this
   target does not exist. It keeps the names of `create_temp`. **Its rename cannot replace:** Linux `renameat2` with
   `RENAME_NOREPLACE`, macOS `renamex_np` with `RENAME_EXCL`, Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`, so a
   program that creates the target during the copy keeps its file, and the item fails `already_exists`. This costs what a
   rename costs. `unsafe` stays in `mmap.rs`, so these come through `tempfile`'s `TempPath::persist_noclobber`, which
   falls back to a hard link on `EINVAL` and `ENOSYS`: `tempfile` is a dependency of `hikasa-vfs` for this one call, and
   brings no new crate into `Cargo.lock`. A file system with neither (FAT) looks, then renames: only there a racing creator
   can get in between. `LocalWriter` commits a new file the same way.
9. **`copy_new_file` needs one back-end and no `create_parents`:** a root of `cp --parents` takes the next path, with the same
   durability. An `mtime` a `SystemTime` cannot hold answers `Ok(None)`, for `filetime` to try.
10. **Backups.** `BackupNames` is additive: it is W6b's call for "names are final at planning", with the numbering and the
    `is_a_directory` of a folder at the name, and A6.6 tests it here. The planner calls `reserve` for every target and source
    of the plan before the first `assign`; without that, only the items assigned so far are known. `BackupPolicy::target`
    numbers a name that would be the item itself, in every style (`abc.bak` with `stem_bak` gives `abc-2.bak`; the item's own
    folder with `folder` gives `x-2.txt`), or the rename would lose the file. With `--no-preserve` a regular target keeps its
    mode across a backup by a `set_attrs` after the commit: the writer finds no old file to copy it from. A link or a special
    file hands on no mode: the new file has the default, as without a backup. `write_backup` of a link goes through a
    temporary sibling, to be atomic. `replace_with` without `RENAME` takes `write_backup` first. Its old backup is replaced
    only once the new item stands. `rename` with a backup goes through the same `with_backup` as a copy, and a `rename` that
    meets another disk expands to a `move` that keeps its backup and its `clear_read_only`; in a batch that expansion
    replaces, because the item says the plan decided so (§1.6.2 had `MoveOptions::default()`, which would fail it).
11. **`trash_fallback` is a field only.** Its execution is §1.6.12, W6b, with `WarningCode::TrashFallback`.
12. **`extract_to_cache`.** A name over 255 bytes does not fit one path component, so it counts as unsafe. The reservation
    may pass `cache_max` by what readers hold, or a zip member over half the limit could not be extracted while its own spool
    is open; the limit stands again when the reader lets go (`Cache::trim`), and only spools can be evicted for it. A sweep
    clears `R` in `x*` folders of a dead session.
13. **`Entry::special` inside a batch.** `Overlay::entry` does not carry it, so a device member reads `None` while a batch is
    open. Nothing filters there.
14. **`attrib` text** is written in lower case, `+hr-s`, and read in any case, as `parse_letters`. `unix_type` letters are
    lower case only, as the contract lists them.
15. **`handling` keys are skipped when they hold their default,** as `ignored` and `format_overrides`, so `to_toml` of old
    rules, session files and goldens stay byte for byte.
16. **A `NotScanned` orphan stays an orphan** (M1's `--no-scan-orphans`); only a pair is Unknown. A child orphan of a refreshed
    pair stays `NotScanned` with auto-scan off: "directly in a base" holds there too.
17. **`--expand-differences` works alone,** as *only expand subfolders with differences* would; it needs no `--expand-on-load`.
18. **`FolderStats::ignored`** counts rows whose category the mark changed, and is left out of JSON at 0. It walks the tree when
    there are marks (O(nodes)) and is 0 without: a counter would have to follow every content result for a figure only a
    statistics dialog shows. Marks are placed in `Scan::insert`, not `plan_pair`: that is where a parent's path is known,
    once per folder.
19. **Modules.** `refresh.rs` and `content.rs` are `scan/refresh.rs`, `scan/content.rs`: they need the scanner's private types,
    and the architecture test bans `#[path]`.
20. **Ids no longer rise from parent to child.** A kept folder moved to the tail stands above its children, so INV-FOLDER-1
    loses "parent is older" for "every live node is reachable once", and `canonical::build`, the recounts and the reference
    rows walk `Store::post_order`.
21. **A kept row keeps its old record whole;** a kept folder's own time is not updated, as `to_canonical` has none. A row
    `CONTENT_PENDING`, or a folder left `NotScanned` that a scan would enter, is not kept. **What a stop leaves is in the
    tree:** `internal::UNLISTED` is on a folder with a listing to come (from its row's planning until the listing is in) and
    on what a refresh was asked for and did not reach, `CONTENT_PENDING` on a row with a result to come. A refresh of a model
    that is not complete goes down to all of these below what it was asked for, because a kept row is not entered, and the
    verdict comes back when no live node has either; a refresh of another subtree leaves it `incomplete`. The check walks
    the arena, and only after a stop. An opened archive whose `mtime` moved counts as changed (*a new stamp below one*).
22. **Refresh and followed links.** A folder followed through a link is one folder at two paths, and a refresh lists the one it
    is asked for. P-REFRESH runs without following; §1.6.4 rule 1 keeps operations from writing through them.
23. **A refresh of a `NotScanned` folder opens it,** by §1.6.7, so a refreshed tree can hold more than a fresh one with auto-scan
    off. P-REFRESH refreshes scanned folders only.
24. **On-demand tests leave the state alone until the outcome.** `CONTENT_PENDING` is set and cleared, the stored state stays,
    so a cancelled run has nothing to restore. `apply_content` also clears `CONTENT_SKIPPED` and `DiffFlags::CONTENT`, or a
    repeated test would keep a difference it no longer finds. A run claims the model like a refresh: `RefreshBusy`.
25. **"Visible contents" of `compare_contents`** is, until W6b's `select::resolve`, every file pair below a folder with neither
    side hidden. W6b should route it through that one definition.
26. **`compute_crcs` runs in sequence on the calling thread.** It is one read per side, and a back-end's CRC needs no read.
27. **D-164's figure.** At the `ops` level, stamped batch with fence against the plain copy of §1.6.22: **1.48 ×** (363.6 ms
    against 244.9 ms, median of three, release build, `/var/tmp`, 10,000 files of 4 KiB in 100 folders;
    `core/hikasa-vfs/tests/perf_copy.rs`). G7 is W6c's, through the Service.
28. **A listing that keeps failing.** `ReadDir` on Windows answers each `next` with the error of a folder that went away, so
    `.flatten()` over it never ends. The walks in `cache.rs` and `lock::reap` now stop at the first failure
    (`map_while(Result::ok)`). A sweep clears `R` in the `x*` folders of a dead session only; a session's own `cleanup` has
    removed them, so its exit lists nothing more than it did before W6a.
29. **`extract_to_cache` after a program deleted its file.** The table entry leaves with what it counted, so the next call
    extracts anew at a path that exists, and a cache does not fill with files that are gone.
30. **Hidden siblings have fixed names:** `.hikasa-xchg[-n]`, `.hikasa-old[-n]`, `.hikasa-tmp[-n]` (of a backup's link), not
    `.<name><suffix>`, which made an exchange fail on a name near the limit of a file system while a copy worked. A leftover
    no longer shows whose it is by name; the item that left it names it.
31. **A move's source is checked again** after the copy, on the plain path too, as `remove_moved` does: the same kind, size,
    time and identity, through a link the comparison followed. A changed source stays, the copy stays, and the item fails
    `changed during the move; the copy was kept`, which is D-163 for what changed during the copy. It costs one `stat` a move.
32. **`..` (D-06).** `LocalPath::parent` of a path that ends in `..` is `None`: its text does not say what is above it, and
    `/a/../..` made every walk up endless, with no token check (`vfs cp -r src a/../copied` hung, since W3). So a spec with a
    `..` is below no base but one spelt the same way up to there (`within`), which is the safe side, and the walks of §1.3.4
    and the fence end. `..` is not removed: **the OS's meaning is kept**. A target or source spelt with `..` is checked for
    nesting by what each level is on disk (`LocalPath::up`, `FileId`, to the root, which is its own parent), so a folder
    cannot be copied below itself by spelling, through a link either. Windows has resolved `..` at parsing, as ever.
33. **The fence and the tree.** §1.6.4 is the rule. The walk replaces the text walk for items, so the two cost one: the folder
    of the last item is compared, and a batch of new files in one folder walks once. Measured on the path of note 27 before
    and after: no change beyond the noise.
34. **Names on disk.** `Vfs::sync_folder` (additive, a no-op but on a disk) syncs what no writer does: a moved link's folder; the
    parent of each folder the operation made (`Exec::new_folders`), once, before the first source that depends on it goes;
    and in an `exchange` the folder of `T2` before step 3 and the target's before step 4, because across two disks a rename
    could reach one disk with the copy that keeps its old content not yet on the other. `Durability::FileOnly` on `T1`, `T2`
    stays for the files. A move into new folders therefore syncs file, folder and each new name, which A6.3 counts.
35. **A plan refused as a whole** (§1.6.5) reports `failed` on the items at fault and `not_run` on the others: no item ran, and
    no archive was opened. It is checked only when the plan names a backup.
36. **`set_ignored` while a scan runs.** A scan and a refresh keep a copy of the marks; `Store::marks_set` tells them it is old, and
    each insertion then re-marks its rows under its own write lock, so no row falls between `set_ignored`'s walk of the tree
    and the insertion. A scan that sees no change pays one comparison per folder.
37. **`./!x`** was chosen over a backslash escape because `.` is no file name: no list that exists holds such a path, so old
    lists read as they did. Only a first name starting with `!` takes it; `a/!x` needs none.

**W6b.** Where the implementation interpreted or extended §1.6.9–§1.6.13, §1.6.15–§1.6.19 and §1.6.21, and why.

1. **`Resolved::depth` and rows of `resolve`.** `Resolved` gains `depth` (additive): post-order, and the `remove_dir` of a folder
   after its contents, come from one pre-order pass over the list. The list holds every row a selection contributes, not
   only roots. The root is never a row: selected, it passes its rows down, so no `keep_dir` or `touch` of a base can be planned
   (note W6a 7). A row selected by itself counts whatever hides it; the builder gives a filtered one `skip filtered`.
2. **Rule 2 of §1.6.9 reads as docs/04 §8 has it: "some of its children".** A folder selected with all it shows on that side
   selected too is the folder. Read literally, `--select all` (which selects each row) would make every folder "a folder
   with a row below it", so `delete --select all` and `move --select all` would leave the empty folders behind, and
   `--include-hidden` would change nothing. What is hidden does not count for "all", and `whole` still decides what may be
   one item. `whole` of a folder is off for a hidden, filtered, failed or `NotScanned` row present **on that side**; a row only
   on the other side leaves nothing out here.
3. **Matching a folder: rows the side does not have play no part.** "On the same side … every shown row below it matches" is read
   per side. Read as every row, `left.all` would refuse a folder with one right orphan, and a collapsed folder could not be
   selected by `all`. A folder with no row of its own side shown is empty: `all`, `orphan`, `exact`.
4. **`diff` of §1.6.9** takes `UnimportantOnly`, `ImportantDifferences`, `BinaryDifferent` as "a content result that differs".
5. **`--path` and a same-named row.** A file and a folder of one name are two rows (D-20), and `node_at` finds the first. `--path`
   takes the sibling that has the side asked for.
6. **Depths.** A folder that exists gets no item (rule 3), but a move's `remove_dir` must see what failed below it, so the items
   below a source folder that goes are one level deeper than its `remove_dir`, with no `keep_dir`. `exec.block` then does its
   work, and rule 4 holds as written. A depth is a count of levels and nothing more: the rows of a folder that exists follow
   a sibling's at the depth of that sibling's contents. So execution cuts by path, not by depth (note 30).
7. **A skipped folder with a source holds items.** `Confirmation::finish` turns a declined `create_dir` into `Action::Skip`, and
   `run_item`'s cut looked at the action, so what lay below ran into a folder that was not made. `execute_with` now cuts below
   a `Skip` of kind `Dir` with a source (`holds_items`); a skipped `remove_dir` has none, and what follows it is no child of it.
   Since the review the plan itself skips what lies in such a folder (note 30), so the cut finds nothing left to stop.
8. **`same_file`.** A replacing item also runs `check_nesting` (with `check_move_inside`, `check_same_archive` for a move): one
   `stat` of the source, on items that sync a file anyway. The model cannot see that two bases are one folder, through a link
   too, and a move onto itself would lose the file. `check_nesting`, `check_move_inside`, `check_same_archive`, `spec_key`
   became public for this and for rule 16.
9. **Rule 13 and rule 5, followed rows and link members.** An exchange skips a pair with a followed side (`followed_link`): there
   is no file to swap on a side that is a link to elsewhere. A pair of links with a member on a side is `skip symlink`, with
   `archive_symlink_skipped`, or a hostile archive's link would land on a disk (D-142). A move of a followed folder is `skip
   followed_link`: its contents cannot go (fence rule 1). Rule 5 as written skips every link member, where W3's planner keeps
   a link between two archives; the rule stands, for rule 6's listing too.
10. **An opened archive that goes whole goes as the file it is.** A row of an opened archive, `whole`, with no target: `copy`,
    `move`, `remove`, `trash` or `rename` of the file, stamped by one `stat`. As a folder it would copy its members to a
    folder of that name, and `check_item` refuses to remove an archive's root.
11. **Rename order and `Action::Rename` for folders only in a move.** Folders of a rename go deepest first, or a folder renamed
    before its rows would take their paths along (rule 2 above lets both be selected). A file `move` stays `Action::Move`, as
    rule 13 says, also on one disk: it is a copy and a removal, so a large file on one disk is slower than a `rename` would
    be. A later part may plan `rename` under D-97's conditions: `run_rename` keeps the backup, W6a note 10.
12. **`.<a>.hikasa-ren`** falls back to `.hikasa-ren[-n]` for a name near 255 bytes (W6a note 30's lesson). The hidden name is
    numbered around every name of the folder and every target.
13. **Confirmation.** A plan where nothing runs asks no operation question. `finish` declines an item whose question is still
    open, and `Err(Declined)` with the operation's open: what was not answered does not run. `folder_confirmation_strict`
    carries D-77: `confirm_delete` as `true`, for adapters that must not depend on a settings file.
14. **`ConfirmPolicy::from_options`** is the free function `api::folder_ops::confirm_policy`: the type is `hikasa_folder`'s and
    `Options` the session's. `OpPlan` gains `refresh` (the parents of the rows it names): `folder_op_execute` has no model.
15. **`OpReport::expanded`** (additive): where an item grew at execution, so `meta` follows without a clone of each item's spec.
16. **`trash_fallback`.** One warning per operation names the items planned for a permanent delete. `NotFound` and a stop do not
    fall back. `clear_read_only` of the root goes to each expanded item. *Revised by the review:* the expansion is what the
    plan stamped (`PlanItem::fallback`, filled only with the fallback on, from the model or rule 6's listing; a file by its own
    stamp) and nothing lists at execution, so a file added since stays, with its folder, and the item fails. A bin that takes
    the whole folder would have taken it along: that is the bin's to give back, a permanent delete is not.
17. **`escape_mask` and `!`.** `[!]` is no set (`!` after `[` negates), and no one-character set can hold `!`, so a leading `!` stays
    literal: it has no meaning in a mask, and the property holds. `escape_mask` also takes `\\`, `/`, a trailing blank and a
    trailing `.` (which would mean "no extension"). `;` in a set needed lists split outside sets: `mask::split_items`, in
    `MaskList::parse`, `NameFilter::parse` and `from_lists`. A base-level folder is `.\\x\\`: `x\\` would match at any depth.
18. **`folder_contents`** takes `ContentsRequest::text_rules` (additive): the comparison's, else the defaults, as `model` has none.
    `compare_contents` keeps its signature and reads `resolve` with `Include::visible()`.
19. **`FolderSyncOutcome`** holds `op` only until W6c adds `rows` and `counts`, and `sync_log_entries` is `op_log_entries`.
20. **Flat comparisons.** A row's folder is no row, so an orphan's target is `<base>/<dir>/<name>`. *Revised by the review:* the
    folders on the way are `lstat`ed once each and the missing ones planned (rule 3); a rename's collisions and `copy`'s
    `duplicate_target` go by the real folder (`dir`), and `copyto` keeps a row's folders in its structure. A move leaves the
    source's folders, which are no rows. `folder_refresh` gives `report: None`.
21. **Conflicts of rule 6's items.** *Revised by the review:* rule 6 lists at planning and computes every conflict of §1.6.11 from
    what it listed and what an `lstat` found, as for a row, with stamps for each item. `copyto` and `moveto` read the target by
    `lstat`, as it is no row.
22. **`--target x.zip/`.** `FolderOp::CopyTo` and `MoveTo` have `folder`, from a trailing separator of the typed target (`\` too
    on Windows): a missing `x.zip/` is a folder, `x.zip` an archive, as `folder op mkdir`.
23. **`hikasa folder op open-with --associated --dry-run`** prints `associated <spec>` per item; `commands` of the JSON the same.
24. **Session keys.** `read_only_left`, `read_only_right` are always written, as the contract's example has them, so
    `session edit --set specs.read_only_right=true` finds its key. `session_add_excludes`, `session_set_ignored` are the Service's
    writes of `exclude` and `ignored`; `SessionWriteDto` gains `masks`, `ignored`.
25. **`parse_side`, `parse_sides`, `parse_structure`, `parse_confirm_mode`, `parse_exclude_what`, `parse_select_path`, `Reply::parse`,
    `ConfirmMode::parse`** hold the words in core, for check 7.
26. **Figures.** Probe of `folder_op_execute` on 10,000 files of 4 KiB in 100 folders through `plan_op` on a loaded machine, release,
    median of three: **1.45 ×** a plain copy (391 ms against 270 ms), which is the path of W6a note 27: `folder_op_execute` adds
    `execute_with` and one walk of `expanded`. `plan_op` of those 10,100 items takes 12–14 ms, so about 1.3 µs an item. The
    official G7 stays W6c's.
27. **Architecture.** `check_confined_crates` also keeps `execute_with` and `plan_op` to `api/folder_ops.rs`, `ops::execute` to
    `vfs_ops.rs`, in `hikasa-session`; `op_dialogs.rs` joins check 7 with W6d, which adds the file.

**W6b after review.** Decisions of the review of W6b, where the contract was silent or let data go; safety won each time.

28. **`--path` names one row** (D-185). `named_row` refuses an empty path and every spelling that is only separators, `.` and `..`
    (`""`, `.`, `/`, `./`, `left:`, `right:/`), before a row is looked up, and any path that comes to the base after it; one
    function for `op`, `compare`, `crc`, `exclude`, `ignored` and `open-with`. A name of dots (`...`) is a name. `--in` of
    `mkdir` and `--expand` are no `--path`: an empty one is the base there, which selects nothing.
29. **Disable editing and a target** (D-184). `ops::writes_into(vfs, base, target)`: the outermost files of both (a write to a member
    rewrites its archive); equal or below by the text once `.` and `..` are resolved; then by identity: the nearest folder that
    stands on the way to the target, and each `..` above it as the OS resolves it, which follows every link on the way, against
    the base's identity through a link. An error other than "not there" answers yes. Checked in `folder_op_plan` after the target
    is resolved and before the selection, for the CLI, a session and IPC alike. Without file identities (some back-ends) only the
    text speaks.
30. **What a plan shows runs.** Execution cuts below a failed or skipped folder item by path (`Cut::holds`: a target below its
    target; a source, or a `remove_dir`, below its source), so rows of a folder that exists, which follow a sibling at a deeper
    depth, are no longer cut. At planning `skip_below` gives what lies in a skipped folder item (a refusal, rule 8, a declined
    answer, from any source of that folder) the folder's reason, so nothing the plan let through is left `not_run`. An item
    `not_run` with no cancel is no success: `OpReport::ok` and `VfsOpDto::ok` say so (exit 100), and a summary ends with `, <n>
    not run`. This applies to `vfs` commands too, where a folder that changed during a run left its contents `not_run` with exit 0.
31. **Rule 6 lists at planning** (`ops/unlisted.rs`), and W3's `plan_copy`, `plan_move`, `plan_remove` are no longer used by the
    builder. The listing is sorted as a comparison sorts rows (kind, folded name), so "the first of two sources" is the same for a
    listed folder as for rows. Filters are the comparison's own, compiled once at its scan (`Store::resolved`, with its date
    cutoff); an archive inside a listed folder is judged as the file it is, and the display filter has no states to judge, so it
    takes no part. A folder that keeps something stays; a moved folder is one `rename` only when nothing in it stays. `touch` sets
    a listed folder's own time only, as rule 14 touches rows, and an exchange of two listed folders stays `skip compare_error`: both
    say what they do, and neither is a loss.
32. **A folder that replaces a file** has a file's conflicts (time of the folder against the file's, read-only, system, a move),
    and the human line names it: `replace with folder`; a link that replaces: `replace with link`.
33. **A read-only archive.** Each item that changes a member of an archive whose outermost file is read-only (DOS `R`, or no
    write bit) asks `read_only`, one `stat` per file. A yes lets the rebuild run as the platform allows: on Unix the rebuild
    replaces the file by rename and keeps its mode; on Windows the `R` of the outer file is not cleared, and the rebuild fails
    as its replace does, with the archive as it was (not tested here: wine does not model it).
34. **An opened archive moved onto its pair** goes as the file it is and replaces the other side's file, as a file does:
    `replace_during_move` asks, a backup keeps the old one, and the right side's own members go with it. A member-by-member move
    would keep them, but it cannot remove the source's root, and the plan has to do what it shows. `copy` of a pair stays member
    by member, which is what a copy of two folders does.
35. **`overwrite_backup`** (bit 32) is the conflict of a backup whose name a file holds that this plan did not make (D-167 keeps
    one generation): asked under `confirm_overwrite_newer`, declined by `no-to-all`, shown in a dry run, and replaced by `--yes`.
    For an archive's one backup every item that rewrites the archive carries it.
36. **Two sources of one place in `copyto`, `moveto`.** The plan records the folders it makes (`made`, folded as the back-end
    compares names, also those a `rename` takes along) and the file targets it takes (`taken`). A second folder merges into the
    first (`keep_dir`), a file where the plan makes a folder, or a folder where it writes a file, is `skip duplicate_target`,
    and a folder on the way over a file of the plan fails the source. A folder planned only as the way to a source goes when
    nothing goes there, and a way folder that cannot be made fails the source at its own depth, so its folder stays.
37. **Ctrl-C at a prompt.** The CLI reads a person's lines on a thread of its own and looks at the job's cancel token every 20 ms,
    so the first Ctrl-C ends the prompt as `Cancel` (exit 100, nothing written) without a line typed. The thread stays blocked
    in its read until the process ends. Tested on a real terminal through util-linux's `script` (Linux only); the end of the
    input and the replies stay unit tests.
38. **Open With.** `expand_command` gives OS strings, so a name that is not Unicode reaches the program; `%b`, `%x` split by
    `Path::file_stem` and `extension`, which keep D-07's reading. A value of `%n`, `%b`, `%F` or `%P` that begins a word and
    starts with `-` becomes `./-…`; `%f`, `%p` are absolute. A row missing on a side asked for is `not_found` before anything
    starts. `open-with` with neither `--app` nor `--associated` was already a usage error (clap), now tested.
39. **`--select all` takes a folder that failed to list**, and only `all`: the plan shows it `skip compare_error` and exits 100. A
    folder that was not scanned still matches no mask (§1.6.9 "scanned"): `--select all --no-scan-orphans` does not bring
    unlisted folders, and `--path` does.
40. **`mkdir --name 'x\'`** is refused off Windows (`parse_mkdir_name`): a final `\` separates nothing there, and dropping it would
    make another name than the one typed.
41. **P-OPS** compares the tree with a model of each verb (`props_ops_model.rs`), on a case-sensitive `MemoryVfs` with Unix modes,
    files and folders (links and case folding are the conservation property's). It found the bugs of notes 30 and 36, a whole
    rename that took a filtered file along (a row a filter hides now leaves its folder not `whole`, §1.6.9 rule 3), a folder of
    rule 8 that replaced a file a filter hides, and a flat `copyto` that passed over an unlisted folder.
42. **A source that writes nothing claims no place** (P-OPS on CI, seed `a049c850`). Rule 16 drops a folder planned only as the
    way to sources when nothing goes there, and that decides duplicates too: a way folder whose sources all stay (filtered,
    refused, failed) is no folder of the plan, so a file of the other side at its path is no `duplicate_target`; it goes, with
    the folders above it (`holds_folder`). Sides are planned one after the other, so when that file comes every source that
    could still go into the folder has been planned. A folder source of the other side that lands on a way folder makes it its
    own, and it is made though nothing goes in, as a copied empty folder is. The mirror case: a row refused for its own reason
    (`filtered`, `compare_error`, `followed_link`) that a file of the plan stands in the way of is that skip, not an item error
    `not_a_directory`. Which way folders are needed is decided as names compare (folded when the comparison or the back-end
    folds case), so a folder used only by its other spelling stays.

**W6c.** Where the implementation interpreted or extended §1.6.14–§1.6.17 (sync), §1.6.22, and why. Safety won where the
text was silent.

1. **One planner.** The types and `plan_sync` are `hikasa_folder::sync`; the planner is `ops/sync.rs`, which drives
   `ops::build::Builder` (`delete_row` per side, `transfer` per direction, `close_open`, `finish` with `writes = true`), whose
   helpers are private to `ops`. A sync therefore has the builder's conflicts, stamps, filters, fence, backups (`reserve`),
   `same_file` and capability checks; `depths` and `covered` are reset between the four phases, as `delete` does per side.
2. **Rows.** `SyncPlan::rows` has one row per node but the base, in post-order. A pair of folders has situation `None` and no
   reason (`compare_error` when it failed), and is not counted in `leave`. A folder on one side with contents takes the action
   of its rows: all deleted on its side → its own delete (`remove_dir` after them, or one `trash` with the bin, `whole`);
   any copied → `copy_*` (made by rule 3; `whole` when all are, so an opened archive goes as its file); else `leave` with the
   reason of its rows that says most. An empty folder on one side reads the matrix, and its action becomes `leave`, reason
   `matrix`, without `create_empty`. `counts` count every row with an action, folders too; `SyncPlan::folder(node)` is the
   sum of the rows strictly below `node`, and `folder(ROOT) == counts`.
3. **Additive API.** `SyncPlan::{preset, matrix, orphans}` and the same on `FolderSyncOutcome`, whose `dto(read, all_rows)`
   takes the comparison for paths and kinds (a `SyncRow` is its id and 8 bytes of decision: 12 bytes);
   `Service::folder_sync_dry_run`; `SyncAction::{reads, writes, deletes}`, `Situation::{only_on, allows}`,
   `SyncMatrix::set`, `SyncCounts::{add, of_row}`, `SyncOptions::{effective_matrix, validate}`; `FolderSyncRequest` derives
   `Default`; `api::{FolderSyncDto, SyncRowDto, sync_options}`, `folder_ops::{sync_counts_text, sync_marker}`,
   `human::folder_sync_lines`.
4. **Rule 3 is stricter than written.** A folder a filter hides on **either** side hides what lies below it on both: a
   right-only file in a folder the left's filters hide (a protected folder, `exclude_protected`) is no orphan a mirror
   deletes. A row every present side of which a filter hides is no row of the sync ("both sides hidden are not rows"), with
   `--include-hidden` too, which brings the hidden side of a pair only; a name filter hides both sides at once, so it matters
   for the size, date and attribute filters.
5. **Rule 2.** A folder that is not `Scanned` (one a model not made by `sync_folder_rules` left unscanned, or one that failed)
   is `compare_error`, as is everything below a failed folder; a sync never lists at planning (§1.6.10 rule 6 is not used).
6. **Overrides** are keyed as the comparison aligns names (`path_key`, as Ignored marks). An override of the base (empty,
   `.`, `/`, a `..` anywhere) is `invalid_input`, before a scan: `--path`'s lesson. One that names no row gets one
   `switch_ignored`, as one a row cannot take does; `--override` on Windows takes `\` too. An override on a folder decides its
   rows; the folder follows them (note 2).
7. **A followed folder on one side is its link.** When the matrix deletes it, the link goes (fence rule 2) and its rows are
   left `followed_link`; otherwise its rows are copied through it: reading through a link is allowed, writing below one is not.
8. **Freed names, own counterparts** (`Builder::freed`, `Builder::own_only`, off for `folder op`). A copy's target held by the
   entry of a row this plan deletes is free: no target stamp, no `replaces`, which the order of D-182 makes true; a delete
   declined at the question leaves the copy to fail `already_exists`. A copy replaces only its row's own counterpart, never
   an entry of another row: Update Right of a left folder `y` onto a right file `y` (two rows, D-20), or of `b.txt` onto a
   `B.txt` that a case-sensitive comparison keeps apart on a file system that folds, fails `already_exists`, and the file
   stays: Update never removes. Found by P-SYNC's model.
9. **Refusals.** ~~"The other base has no item at all" counts every entry, filtered or not~~ (rejected by the review: a
   base is empty when it holds no file after filters, *after review* 6), and refuses only a plan that deletes on the other
   side. An unfinished comparison refuses a plan that deletes; one that only copies plans, its rows not
   reached being `compare_error`. The stored `[rules.sync.matrix]` is validated whatever the preset, so `session edit` and an
   import refuse a bad cell before `custom` could put it in effect.
10. **Words.** `--custom`'s base is the stored matrix, else all `leave`; `custom` is no base word. `--recycle-fallback` needs
    `--recycle`. `--select`, `--path` without `--just-selection` are refused (exit 100) rather than passed by; `--just-selection`
    that selects nothing is refused as in `folder op`. `--visible` shows what `--display` (default `all`) shows. A preset is
    needed unless a folder-sync `--session`, whose `[rules.sync]` is the base (a folder-compare session needs one: see
    *after review* 3). The human lines
    leave out `same` rows, `--all-rows` too (JSON has them), and a row left says ` (<reason>)`.
11. **`session run`.** `SessionRunRequest::Sync { request, options }` (additive); its read-only sides come from the session.
    `--yes`, `--confirm`, `--dry-run`, `--allow-empty-source`, `--all-rows` on another type are refused: a comparison would pass
    them by in silence. `folder op --session` takes a folder-sync session's folder rules (`SessionRunRequest::folder_request`).
12. **Sessions.** `RulesShape::SYNC` is `FOLDER` with the map `sync.overrides`, so an override removed in memory leaves the
    file. A folder-sync file written before W6 had an empty `[rules]` (nothing could add a key to the opaque table), which
    reads as the defaults; a hand-made one whose typed key has another type is now `invalid_data`, as for folder-compare.
    Unknown keys stay (D-70).
13. **Confirmation.** `folder_confirmation(plan, None)` of a sync takes `sync_confirmations`; the CLI passes its mode to
    `folder_confirmation_strict`, so a script never depends on that key (D-77).
14. **IPC (D-190).** `OpenOptions::{no_backups, sync: Option<SyncWords>}` are left out of the wire while off, so the wire
    goldens and older peers see the same messages. `open_view_request` makes `ViewRequest::Sync` of a `folder_sync` view, the
    words parsed by `api::vocab::sync_options`, the CLI's parser; `check_open` checks a sync before any contact. The server and
    the GUI answer `unsupported_view`, "this window cannot show a Folder Sync view yet" (`ipc::unserved_view`); the error kind is
    `unsupported`, exit 100. The GUI's arms of `ViewRequest::Sync` are never reached; were one, it would show the comparison of
    the bases, which writes nothing. W9 replaces them. `--sync` and `--nobackups` are no longer forwarded with `switch_ignored`.
    `folder sync --gui` sends the sync in effect (a session's `[rules.sync]` with the flags on top) as words,
    `api::sync_words_of`, which `sync_options` reads back to the same options, and `--read-only`, `--no-backup` in
    `OpenOptions`. An `OpenView` carries no base rules, so with `--session` the session's folder rules do not travel (as for
    `folder compare --gui`); W9's view should open a session by `LoadSession`.
15. **Log.** `sync_log_entries` adds, after the operation's summary, a second `Summary`: the counts, `sync_counts_text`, the
    text of the human `#` line (T11).
16. **Archives.** Within archives, a member of a single stream that differs is `leave`/`capability`; a single-stream archive on
    one side stays for the same reason, though its file could go whole (its members cannot be removed one by one). Without
    *within archives* an archive is a file.
17. **G7 copy** failed first: 1.52, 1.58, 1.52 × (load 5) and 1.54, 1.55, 1.53 × (load 1.8) on W6b's path. Two calls per new
    file were repeats: `copy_new_file` stat'ed its source by path a moment after the stamp's `lstat` (now `HandleMeta::size`,
    additive), and `run_item` `lstat`ed an absent target that the rename that cannot replace checks again at the commit
    (W6a note 8); a copy now makes D-103's check in `run_copy`, by that rename on the new-file path (its `AlreadyExists` becomes
    "changed since the plan was made") and by the `lstat` before streaming otherwise. After: 1.39–1.48 × in six rounds (loads
    1.2 and 6). The other extra calls are guarantees and stay: the folder above each item (D-94 rule 3), the stamp, a
    temporary name and its rename. Figures in `docs/02`.
18. **Memory.** The first count of an `OpPlan` of 101,000 items was 91.9 MiB: the vectors kept the capacity they grew to, and a
    `Spec` was 112 bytes for URL variants no plan holds. `finish` now shrinks them, and `Spec::Remote`, `Spec::Http` hold a
    `Box` (a `Spec` is 56 bytes, a `PlanItem` 472): 56.2 MiB, and 57.5 MiB for the `SyncPlan` of that tree.
19. **P-SYNC** (`props_sync.rs`) compares the disk with a model of §1.6.14 written from the tree before and the options alone,
    with its own table of the presets; 384 cases by default (about half run an item), 4,000 once. It also checks the rows'
    decisions, dry-run statuses against the run's, the re-comparison of each copied or deleted row, a refresh against a fresh
    comparison, and that a second run of a preset plans nothing that runs (items below a folder the plan cannot make excepted:
    execution cuts them). It fails when the plan's items are dropped, when Mirror to Right and to Left swap their rows, and when
    the copies' direction is flipped. A6.28 (`props_sync_fence.rs`) fails when the planner's and the builder's followed-link
    refusals and the fence are taken out.

**W6c after review.** Decisions of the review of W6c, where the contract was silent or let data go; safety won each time.

1. **A whole folder's `trash` checks what lies below it** (§1.6.12, `PlanItem::verify_below`): the subtree is always
   stamped, and a name, a stamp or an entry added since fails the item before the bin is called. `folder op delete
   --recycle` and a sync alike, listed by the comparison or by rule 6.
2. **`include_hidden_by_default` is no sync's.** `FolderSyncRequest::include_hidden` is the sync's own flag, `None` = off,
   for rule 3 and for `--just-selection`'s selection; the folder-op preference never reaches a sync. The other options a
   sync reads are those the contract names: `[backups]` (§1.6.5), `preserve_created`, and the confirmation keys
   (`sync_confirmations` for a view, `confirm_*` for the per-item questions, §1.6.11; the CLI asks the operation question
   whatever `confirm_delete` says).
3. **Note 10's default preset is rejected.** `folder sync --session <folder-compare session>` without a preset flag is a
   usage error (`invalid_input`, exit 100), with `--gui` too, before any contact: only a folder-sync session holds a preset.
4. **A copied link keeps its time** (D-94 rule 6, with `Preserve::times`): after `create_symlink`, `set_times` on the link
   itself (no-follow: `utimensat` with `AT_SYMLINK_NOFOLLOW` on Unix, the link opened as a reparse point on Windows, the
   member's time in an archive), so a second sync plans nothing and Update Both does not copy a link back and forth.
   Every back-end of this build can time a link; one that answers `OperationUnsupported` leaves the link made with one
   `capability_missing` warning. A comparison of links by target and kind instead of time is therefore not built: no
   back-end needs it.
5. **Case-insensitive alignment settles.** With `criteria.filename_case` a sync that replaces a counterpart whose name
   differs from the source's (by case, under a folding alignment) plans one `rename` after the copy, to the source's
   spelling, expecting what the copy wrote (the source's size, and its time when times are kept); a copy that did not run
   leaves the old name. `folder op` does not. **Two source names of one key** are not the plan's to choose: a row whose
   source name shares its aligned key with a sibling's and that has no entry of its own on the other side (and every row
   below such a folder on one side) is left at planning with the new `LeaveReason::AmbiguousName` (`ambiguous_name`),
   counted in `SyncCounts::ambiguous` (additive in `counts`), and the `#` line ends `, <n> left as ambiguous names (two
   names align to one)` when there are any. No item is planned, so nothing is skipped at execution, and a second run is
   the same. The pair the comparison aligned goes on as usual.
6. **Empty sources, wider (note 9 rejected).** A source base is empty when, the filters applied, it holds no file at all:
   only empty folders (a `lost+found` of a mount point that failed), only entries a filter hides (name filters never reach
   the model; size, date, attribute filters mark them; a folder hidden on that side hides its contents). A followed link
   and an opened archive count as files; a folder the comparison could not read counts as holding files (it is unknown,
   not empty, and its rows are left by rule 2). `EmptySource` then refuses a plan that deletes on the other side, unless
   `allow_empty_source`. **Folders emptied entirely**: a pair of folders whose source side holds no such file while the
   plan deletes files below the other side's is not refused but listed (`SyncPlan::emptied`, outermost only, tree order),
   in the human output before the items (`# emptied entirely: <path> on the <side> (the <side> holds no file there)`), on
   stderr before the CLI's question, and in JSON as `result.emptied: [{path, empty, emptied}]` (additive). The confirmation
   model (`Confirmation`) does not carry it: W9's dialog renders `SyncPlan` and shows it there.
7. **Unknown keys of a session that runs** (`Service::session_run_request`, so `session run`, `folder sync --session`,
   `folder op --session`): each key this version does not know, found against the session's full document (the typed
   rules keep their own table, so `ignored_keys` alone does not see them), is one `unknown_config_key` warning naming the
   nearest known key, always, not only with `--verbose`. In a **folder-sync** session a key in `[specs]` or `[rules]`
   (the sync's rules, folder and `[rules.sync]`) refuses the run instead, `invalid_data`, exit 100, naming the key and the
   nearest known one: `[specs] read_only = true` meant to protect a side, or `presett`, would otherwise run another sync.
   Other sections and other session types warn only (T13); the keys stay in the file (D-70).
8. **The fast path of a new file trusts no more than the stamp** (note 17's `HandleMeta::size`). `LocalVfs::copy_new_file`
   reads the opened source through `take(size + 1)`: what is not the file the caller's `lstat` saw (a link to a FIFO a
   writer feeds, a file that grew or shrank) cannot stream past it, and a count other than the stamp's is "changed since
   the plan was made", naming the source, the temporary file removed. A read that fails on what is not a regular file (a
   folder swapped in) names the source too, by an `fstat` of the handle on that error path only. Nothing is added to the
   path that succeeds, so G7 is unchanged within its noise (§1.6.22 figures in `docs/02`). Without a size the back-end
   stats the source as before. Not covered: a FIFO swapped in between the stamp's check and the `open`, with no writer,
   blocks that `open` (as on the stream path); a no-follow, non-blocking open needs flags this workspace has no crate for.
9. **P-SYNC after review.** The generator now makes pairs with one side hidden by a size filter, a pair folder hidden on
   one side (protected attributes), links, folders whose listing fails on one side, a case-insensitive alignment with two
   spellings, an Ignored path, *Just selection* of a path, and comparisons stopped part-way; the model knows each rule
   (rule 1 scope, rule 2 failures, rule 3 both ways, Ignored as `same`, conflicts) and the empty-source rule of *after
   review* 6. The second-run check is the model on the tree the first run left, for custom matrices, overrides and
   no-to-all too; presets still plan nothing that runs. It asserts shares of cases that ran (a third of the plans not
   refused), deleted, hid one side, held links, aligned two spellings, failed a listing, had a scope or an Ignored path,
   and refused an unfinished comparison. The mutations of the review fail it (see the review's report).
10. **A folder-sync file of the old shape** (§1.6.15's "reads as it is"; note 12). Before W6 `[rules]` of a folder-sync
    session was kept as written, so a file may hold an older shape; the pre-W6 golden is kept as a fixture
    (`folder-sync-before-w6.toml`) beside the new one. Where the old shape has one meaning now it is read as that:
    `filters.names` as a filter text (`"*.c;-obj\\"`), a string where a list is expected as that one item. What has none
    fails at load, `invalid_data`, naming the key: `rules.mode` (no preset can be told from `"mirror"`), and a value whose
    type is not its key's now.
11. **`IpcHandler::seen`** (additive, default nothing): the server shows a handler every request after the handshake,
    also those it answers itself (an `OpenView` of a kind it does not serve), so a test records the wire of `folder sync
    --gui` instead of reading it back from an error message.

**W6d.** Where the implementation interpreted or extended §1.6.20 and §1.6.24, and why. Safety won where the text was
silent.

1. **Predicates.** `Ctx` gains `row_selected` (one row selected in a comparison that has finished or stopped, with no
   job of the view and no operation under way), `row_on` (the row has an item on the left, on the right) and `writable`
   (Disable editing). `SideWritable(side)` holds when that side is writable and the row has an item on the other side (a
   copy needs a source); `RowSelected` when the row has an item on a writable side. `Busy` covers an operation's jobs,
   so Stop cancels them. `FullRefresh` while an operation is under way does nothing: its outcome and its refresh come
   first.
2. **One task per view.** *Choosing* (the Delete dialog), *Planning* (one job: `folder_op_plan`, then
   `folder_confirmation`, which reads `options.toml`, D-105), *Asking* (the view keeps the plan, the dialog only the
   `Confirmation`), *Executing* (`JobKind::FolderOp`), *Refreshing* (`JobKind::FolderRefresh`). The four commands are off
   for all of it, and `start_operation` refuses by itself as well, so neither a second dialog nor a second job can start.
   A plan with no question left (the keys off) runs at once.
3. **A stale plan does not run.** The view keeps the model (`Arc`) and its generation from the command on; the plan runs
   only if both still hold after planning and after the dialog, else a warning notice says so and nothing is written. A
   child's result and an Expand refresh wait for the task, so they cannot move the generation under a question.
4. **Delete dialog presets.** A row on one writable side has that side chosen; a pair has none, also when one of its
   sides is read-only (OK stays off until one is chosen; *after review* 5): *Both* is never a default, nor is one side of
   a pair picked for the user.
   `require_side_selection_in_delete` presets nothing ever. A side the row has nothing on is greyed like a read-only one.
   Delete Permanently plans `recycle: Recycle::default()` whatever the dialog holds.
5. **Confirmation dialog.** Enter is Yes for the operation question and No for an item's. "Do not ask again" counts with
   Yes and Yes to All only (the key's `false` means "proceed"), is written by `Followup::DisableOption` once the last
   question is answered and only when the operation was not cancelled, and changes the next operation, not the questions
   of this one. The recycle bin's question and a sync's (`sync_confirmations`) have no box. A dialog that leaves with a
   question open (the window's close button) is a Cancel. The counts under the question are per action, items and bytes
   (bytes only where items carry a size: a removal has none), then what the plan leaves and what cannot run.
6. **Notices.** Key `op:<view>`. Information when `FolderOpOutcome::ok()`; a warning with the new `NoticeAction::ShowLog`
   (it brings the view's tab and opens its log panel) otherwise, and when stopped (`<caption>: stopped; …`). A plan the
   Service refuses is a warning notice with its log entry. The outcome's entries are `op_log_entries` with the tab's
   caption as source; the notice is not logged again, the summary entry stands for it (D-117). When a plan's dialog
   appears, the view's tab comes to the front: the question is about its row. An operation's notice stays when its tab
   closes (what happened on disk is still true).
7. **Rows a refresh replaced.** `renumbered` moves the open folders and the selection; a row whose facts changed gets a new
   record, which is no renumbering (a copied orphan becomes a pair), so the open folders and the selected row are also
   found again by path, where the user changed nothing while the refresh ran (*after review* 3). A refresh that answers
   flat (`report: None`), or fails after an operation, runs the comparison again; so does one that leaves
   `garbage() > len() / 2`.
8. **Full re-runs keep what is open by path.** `FullRefresh` and the re-run of note 7 find the open folders and the
   selected row again by path once the new comparison ends (the row only when none was selected meanwhile); before, the
   old ids were kept and could name other rows of the new model. "A `FullRefresh` is unchanged" is read as: it still runs
   the comparison again.
9. **Open on a closed archive** runs `folder_open_archive` as a job; once it opened (the job succeeded and the row is a
   folder that did not fail to list), the path joins the view's `request.patch.open_archives`, which `duplicate()` gives
   the session, and the row opens by path. An archive that cannot be read is not remembered (*after review* 7).
10. **FC-012.** `expanded_on_load` is applied when a run ends (not with *expand all*, nor after a re-run that restores
    paths). Expand, or the disclosure triangle, on a `NotScanned` folder refreshes it, then opens it; while a task of the
    view runs it waits, by path, and runs when the task ends (*after review* 6).
11. **FC-029(c).** The ticket is the row's path and both stamps, not its id: ids move and records are replaced. The result
    goes to the live file row at that path, through `apply_child_result`'s stamps, once per child (its first result), with
    `ContentMethod::Rules` and the child's `verdict(false)`; it waits while the view's task runs; a child closed first
    gives none; a re-run forgets the tickets. A child opened through its parent's session (a store job) is adopted when
    `finish_open` has its id.
12. **Disable editing and `--nobackups`.** From `OpenOptions` on every `open_view` (IPC and the command line; `--ro` is
    both sides), from the session's `[specs] read_only_*` on every session open, and copied by `TabDuplicate`. A view's
    session carries its read-only sides, so Save, Save As, auto-save, the Settings dialog and a workspace keep them, and
    Session Settings OK takes them back from that session; `--nobackups` comes from `OpenOptions` only and is never
    written to a session, but survives Settings OK, `TabDuplicate` and a workspace (the tab's `view` table, key
    `no_backups`). See *after review* 1.
13. **Tests.** `FolderViewModel::gate` (`cfg(test)`) holds the plan, execute, refresh and archive-open jobs, and
    `scan_gate` a comparison after it scanned, before it reports; `AppCore::before_wait` opens a gate once Quit waits;
    so Stop, the busy states, a click during a refresh and an operation whose view ends are tested without timers. No test calls a recycle bin: a delete to the bin is checked by its plan and cancelled, the
    no-bin question on an archive member is answered No, and the pointer test unticks the bin and checks the model before
    OK. The child's result is tested with a CRLF pair (`rules_same`): a plain text file's inner whitespace is important.
14. **Architecture.** `op_dialogs.rs` is in check 7's list of model files. `folder_view.rs` names no toolkit either, but
    stays out of the list as before.

**W6d after review.** An independent review proved seven defects and fourteen untested behaviours (mutations that
survived the suite); the maintainer decided the fixes, safety first. Where the text above, a help topic or a document
said otherwise, it now says this.

1. **The write policy survives every route.** Before, `AppCore::capture` built a view's session from `Specs::default()`,
   so Save and Save As wrote a protected session without `read_only_*`, and Session ▸ Settings ▸ OK reset the view to
   all-writable, even with nothing changed: Copy to Right then overwrote a `--ro2` side. Now `ViewCapture` carries the
   read-only sides and its session sets `[specs] read_only_*`; Settings OK takes them from that session (the dialog has no
   control for them, so they come back unchanged) and keeps the view's `--nobackups`; a text view keeps its sides too
   (it writes nothing yet, but its session and its duplicate keep them). A read-only side becomes writable only by an
   explicit change of the session (`session edit --set specs.read_only_right=false`, or the file). `--nobackups` has no
   session key (note 12); a workspace keeps it as `no_backups = true` in the tab's own `view` table (docs/03), read only
   when a workspace tab is restored, never from a session file. A child Text Compare of a protected folder view does not
   inherit its sides yet: nothing writes in a text view before W10, which must pass them.
2. **The question names its row.** The confirmation dialog shows the row's path and its sides (`a/b.txt: from the left
   to the right`, `…: on both sides`) above the question; the Delete dialog already showed the path and what each side
   holds. A selection that moved off the row between the command and the question (while the plan was made, or while
   the Delete dialog was open) cancels the operation before anything is asked: a warning notice "the selection changed
   while the operation was prepared; nothing was written". The same holds for a plan with no question left (the keys
   off): it acts on the row it was given only while that row is still selected.
3. **A click during a refresh wins.** A refresh (after an operation, an Expand, an archive) finds again by path only what
   the user left as it was while it ran: a folder still open (one closed meanwhile stays closed) and the selection, unless
   the user selected a row meanwhile (`selection_moves` counts the user's selections). A full re-run likewise keeps a row
   selected while it compared. A folder opened during a refresh follows `renumbered`.
4. **An operation whose view ends is reported.** Closing the tab, IPC `CloseView`, a workspace load, Session ▸ Settings
   ▸ OK (the model is replaced) and Quit end a view's model through `FolderViewModel::retire`: an execute job is cancelled
   and kept by the window until it has ended its current item (the cancel is cooperative, per buffer); its outcome goes
   to the log with the tab's caption as source and one notice, `<caption> in <tab> (closed): …` or `(settings applied)`,
   without *Show log* (the view is gone). An outcome or notice the view had not handed over yet is reported the same way.
   Quit waits for such jobs before the Service shuts down, at most 30 s (`OPERATION_FLUSH`), so no temporary file is left;
   one still running then is logged as an error. A plan or a question that had not run writes nothing and is dropped.
5. **A pair has no preset side** in the Delete dialog, also when one of its sides is read-only (note 4): before, the
   writable side was preset, and Shift+Del, Enter, Enter deleted it permanently, against `help/dialog-delete.md`.
6. **An Expand during a task waits.** Before, an Expand on a folder not scanned while an operation or a refresh ran was
   dropped. It is kept by path and runs when the task ends (note 10).
7. **A failed archive open is not remembered.** Before, `open_archives` and the row to open were set before the job ran.
   Now they are set once the archive opened; an archive that cannot be read (its folder failed to list, or the job
   failed) gives a warning notice `Open <path>: <why>` and leaves the session as it was.
8. **Tests the mutations asked for**, in `tests_folder_ops_fixes.rs`: a delete of one side of a pair deletes only that
   side; the plan request carries the read-only sides and the Service's refusal shows as a warning; the view refuses a
   copy into a read-only side by itself; "Do not ask again" then Cancel writes no key; a dialog closed mid-questions runs
   no item, not even those without a question; a folder opened during a refresh stays open through `renumbered`; a model
   changed while planning asks nothing; FullRefresh waits for an operation; nothing operates while a comparison runs;
   the window passes `require_side_selection_in_delete`; a child's result waits for an open question; the read-only and
   backup questions through the window, the move and system questions through the dialog. The "second operation
   refused" check of `tests_folder_ops.rs` now looks at the job list and the dialog stack, so it can fail.
9. **The CHANGELOG** keeps one W6 section in `## Unreleased` instead of one entry per part (§1.6.24 says so now): W6
   ships as one pre-release, 0.1.92, and entries per part repeated each other and told of states a later part or a
   review changed (the Delete dialog's presets, what a refresh keeps), which a user of 0.1.92 never sees.

### 1.7 W7 — Reports

**Ids.** RPT-001 (links to file reports; the script and CLI entry points; the Session menu commands are W13's), RPT-002
(XML dumps of text and folder comparisons, clipboard, printer per Q6, HTML wrap), RPT-003 (the options model with
remembered choices and *Just selection*; the dialog is W13's), SCR-001 (the whole vocabulary: columns, strike-out, patch
styles, output options), RPT-004, RPT-010 and RPT-011 extended to the new layouts, TP-003 (done, unchanged), the W6
defect of FC-072 carried over (§1.7.2), gate G13. Decisions D-193…D-212. Out of scope: the report dialog, *View in
Browser*, *Print Preview* and Session ▸ *<Type> Report…* (W13; `Report` and `FileReport` stay `planned` in the
catalogue); FC-079 *File Compare Report…* over a selection (P1, D-195); table, hex and picture reports (W12); Text Merge
reports (M3); native printing (D-62); the columns `column-vcs`, `column-revision`, `column-version` (FC-028 is P1, VCS
P2); the script interpreter (W8).

**Files.** `core/hikasa-folder/src/ops/` (`build.rs`, `copyto.rs`; §1.7.2); `core/hikasa-report/src/` (`lib.rs`,
`options.rs`, `error.rs`, `meta.rs`, `style.rs`, `escape.rs`, `text/mod.rs`, `text/html.rs`, `text/sbs.rs`, new
`request.rs`, new `xml.rs`, new `links.rs`, new `text/interleaved.rs`, new `text/xml.rs`, `folder/mod.rs`,
`folder/html.rs`, new `folder/xml.rs`, new `summary_counts.rs`); `core/hikasa-report/tests/` (new `request.rs`,
`golden_interleaved.rs`, `golden_xml.rs`, `xml_schema.rs`, `links.rs`, `stream_memory.rs`, `props_reports.rs`, new
fixture files); new `docs/schemas/hikasa-report-1.xsd`; `core/hikasa-session/src/` (`api/report.rs`, new
`api/report_write.rs`, `api/requests.rs`, `api/dto.rs`, `api/mod.rs`, `api/human.rs`, `clipboard.rs`, `host.rs`, the
`state.toml` model, `paths.rs`); `core/hikasa-session/tests/architecture.rs`; `cli/hikasa-cli/src/` (`args.rs`,
`cmd/mod.rs`, `cmd/text.rs`, `cmd/folder.rs`, `cmd/session.rs`, new `cmd/report.rs`, `main.rs`);
`gui/hikasa-gui/src/folder_view.rs` (one call moves to core, §1.7.8); `.github/workflows/ci.yml` (§1.7.7);
`core/hikasa-report/Cargo.toml`, `Cargo.lock` (no new package); `help/` (§1.7.19); tests of §6.7.

#### 1.7.1 Parts (D-193)

W7 is one wave in two sequential parts, one implementer each; a part starts when the previous one is committed on
`feat/m2` and ends with the tree green. Nothing runs beside W7.

| Part | Content | Sections | Acceptance | Leaves working |
|---|---|---|---|---|
| **W7a Core** | the W6 defect; `ReportRequest` and the vocabulary; interleaved; XML dumps and their schema; HTML wrap and the print block; link rendering, hrefs and safe names; summary counts in one place; streaming memory | §1.7.2–§1.7.7, §1.7.8 (rendering), §1.7.17 (streaming) | A7.1…A7.10, A7.24 | `folder op copyto` fixed; every new layout through the `hikasa-report` API; the CLI unchanged otherwise |
| **W7b Service and CLI** | pair requests in core; write plans and linked reports on disk; targets (streaming file, clipboard, printer, browser); report time; remembered choices; `ReportArgs`, `hikasa report`, `hikasa.report.v1`; help, docs, G13 | §1.7.8 (writing)…§1.7.19 | A7.11…A7.24 | every layout, target and link headless; scripts (W8) and the dialog (W13) have their model |

The order is forced: W7b's calls render W7a's layouts, and the defect is in `hikasa-folder`, whose P-OPS must be green
before the wave's own properties start. Green is claimed per part (§1.5.19 W5a note 21).

#### 1.7.2 Carried over from W6: `copyto` of an aligned folder pair (D-194)

**The defect.** `folder op copyto --side both --structure base` with case-insensitive alignment, onto a case-sensitive
target, where a pair's folders differ only in case (`l/D/x.txt`, `r/d/y.txt`): the plan made `/o/t/D` for the pair
(note 42 folds way folders as names compare), then gave the right file the target `/o/t/d/y.txt`, its own spelling,
which fails `not_found` at execution. The plan treated the pair as one folder when it made it and as two when it wrote
into it.

**Rule 16 gains a paragraph (normative):**

> **One folder per aligned pair.** With structure `Base` or `Relative`, the target folder of a folder pair whose two
> names differ (case under case-insensitive alignment, NFC/NFD under `align_normalization`) is chosen once for the pair,
> whichever sides are sources: (1) a folder the target holds whose name equals the left's name, else (2) one whose name
> equals the right's (exact names; one `lstat` each, memoized per target folder like rule 3's), else (3) the folder this
> plan makes for the pair, named as the first source side in plan order spells it (left before right). Every item below
> the pair, of either side, is placed under that folder's spec; `made` (note 36) records the pair, not a folded name. A
> pair whose names are equal plans as before.

| Target back-end | At the target before | Sources | Plan |
|---|---|---|---|
| case-sensitive | nothing | both sides | `mkdir /o/t/D`; `x.txt` and `y.txt` into `/o/t/D` |
| case-sensitive | `/o/t/D` | both sides | no `mkdir`; both into `/o/t/D` (a second run lands where the first did) |
| case-sensitive | `/o/t/d` | both sides | both into `/o/t/d` (step 2) |
| case-sensitive | `/o/t/D` and `/o/t/d` | both sides | both into `/o/t/D` (step 1) |
| case-sensitive | nothing | right only | `mkdir /o/t/d`; `y.txt` into it |
| case-sensitive | `/o/t/D` | right only | `y.txt` into `/o/t/D` |
| case-insensitive | nothing | both sides | as before W7: `mkdir /o/t/D`, both into it |

Unchanged: file pairs (each source keeps its name; two sources of one target path are `duplicate_target`, rule 16),
structure `None`, and `copy`, `move`, `exchange` (rule 2: the counterpart's own spec). `moveto` follows `copyto`; a
whole-folder `rename` (rule 7) of a pair's side onto a folder chosen by step 1 or 2 is not possible (the target exists),
so that side goes item by item, as rule 7 already says for an existing target. Tests: A7.1.

#### 1.7.3 Scope of the report ids (D-195)

Where the documents disagree, what wins and why:

| Where | Problem | Reading |
|---|---|---|
| docs/16 M3 "reports for every view incl. XML and linked file reports" vs RPT-001, RPT-002 (P0) and plan W7 | XML and links are P0 and planned in M2 | P0 and the approved plan win; the roadmap line becomes "reports of the P1 views (table, hex and picture side-by-side and interleaved; media, version, registry)" |
| RPT-001 "File Compare Report… for the selected pairs" (P0) vs FC-079 (P1, docs/04) | the same command, two priorities | FC-079 wins for the command over a selection: docs/04 owns the folder view's commands, and no plan wave lists it. The machinery is W7's (§1.7.8), so FC-079 in M3 is a dialog over it. The script's `file-report … <left> <right>` is a text report (W8) |
| RPT-002 "HTML … wrap none/word" vs docs/13 M1 "HTML always wraps by CSS" | M1 knew one wrapping | RPT-002 adds; M1's (by character) stays the default with unchanged bytes (D-210) |
| SCR-001 `line-numbers` vs the CLI's default *on* | no word turns them off | `no-line-numbers` is added to the vocabulary (D-196) |
| RPT-003 display "None" vs SCR-001's `display-*` | no word | `display-none` is added: no row, only the header and one hidden-rows line (text), no item (folders) |
| TP-003 is P1 | implemented in M1 | stays done; W7 changes no patch byte |

#### 1.7.4 `ReportRequest` and the vocabulary (D-196, D-209, D-210)

```rust
// hikasa_report::request (pure; T11)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ReportKind { Text, Folder }                                  // W12 adds Table, Hex, Picture
/// The words of one request, as a script, the dialog or the CLI gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportWords {
    pub layout: Option<String>,          // script `layout:`, CLI --layout/--report
    pub options: Vec<String>,            // `options:` (comma-separated on a script line and on the CLI)
    pub output_options: Vec<String>,     // `output-options:`
    pub title: Option<String>,           // `title:`, --title
}
/// What an adapter means by an absent word; the CLI's values are M1's.
#[derive(Debug, Clone)]
pub struct ReportDefaults {
    pub layout: &'static str,            // `report`: "side-by-side"; `text compare`: "unified"; `folder compare`: "list"
    pub text_display: DisplayMode,       // the CLI: all (human), as resolve_text_display
    pub folder_display: DisplayOptions,  // the CLI: --display/--folders/--suppress-filters, default differences + structure
    pub line_numbers: bool, pub context_lines: u32, pub width: u16,
}
bitflags! { #[derive(Serialize, Deserialize)] pub struct StrikeOut: u8 { const LEFT = 1; const RIGHT = 2; } }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Orientation { Portrait, Landscape }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrintBlock { pub orientation: Orientation, pub mono: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineScope { pub side: Side, pub first: u32, pub last: u32 }   // 1-based, inclusive
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportScope { All, TextLines(LineScope), FolderRows(Selection) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct FileLinks { pub text: TextReportOptions }   // §1.7.8
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportBody { Text(TextReportOptions), Folder { options: FolderReportOptions, links: Option<FileLinks> } }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRequest {
    pub kind: ReportKind,
    pub body: ReportBody,
    pub css: Option<String>,             // `html-custom:<css>` as written: a spec the Service reads (core does no I/O)
    pub print: Option<PrintBlock>,       // `print-portrait|landscape`, or a printer target
    pub title: Option<String>,
    pub scope: ReportScope,              // set by the Service from --lines/--select/--path, never from words
    pub unused: Vec<String>,             // accepted words the layout does not use: one warning `report_option_unused`
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind { Stdout { ansi: bool }, File, Clipboard, Printer, Browser }
impl ReportRequest {
    pub fn parse(kind: ReportKind, words: &ReportWords, defaults: &ReportDefaults) -> Result<ReportRequest, ReportError>;
    pub fn check_target(&self, target: TargetKind) -> Result<(), ReportError>;   // the matrices below
    pub fn words(&self) -> ReportWords;                                          // canonical; parse(words()) == self
    pub fn ignore_unimportant(&self) -> bool;                                    // for the comparison's request
}
/// The one entry point of a request (additive); the M1 functions remain for their callers.
pub enum ReportInput<'a> { Text(&'a TextComparison), Folder(&'a FolderComparison) }
pub fn render_report(input: ReportInput<'_>, req: &ReportRequest, meta: &ReportMeta, style: &HtmlStyle,
    links: Option<&dyn PairLinks>, out: &mut dyn Write) -> Result<Vec<Warning>, ReportError>;   // links: §1.7.8
```

`TextLayout` gains `Interleaved` (`interleaved`) and `Xml` (`xml`); `FolderLayout` gains `Xml`. `TextReportOptions` gains
`strike: StrikeOut`; `DisplayMode` gains `None`; `HtmlStyle` gains `print:
Option<PrintBlock>`; `ReportError` gains `Conflict { slot, first, second }` and `TooLarge { limit }` (`invalid_input`,
`too_large`). Struct literals of other crates break and are updated; the M1 functions `render_text_report*`,
`render_folder_report*` keep their signatures and behaviour.

**Words (normative).** ASCII case-insensitive, `_` equals `-`; a word fills one slot. The M1 parsers (`DisplayMode::parse`,
`Wrap::parse`, `OutputFormat::parse`, `PatchStyle::parse`, `TextLayout::parse`, `FolderLayout::parse`,
`FolderColumns::parse_list`, `FolderDisplayFilter::parse`) are the slot parsers, so every M1 spelling stays valid.

| Slot | Words | Kind | Used by layouts | Absent |
|---|---|---|---|---|
| layout | text: `side-by-side` (`sbs`), `summary`, `interleaved`, `patch`, `statistics` (`stats`), `xml`, `unified` (the annotated diff); folder: `side-by-side`, `summary`, `xml`, `list`, `json` | both | — | `defaults.layout` |
| display | text: `display-all`, `display-mismatches` (`-differences`), `display-context`, `display-matches` (`-same`), `display-none`; folder: every `FolderDisplayFilter::parse` word with `display-` (the SCR-001 folder filters: `display-no-orphans`, `display-mismatches-no-orphans`, `display-orphans`, `display-left-newer`, `display-right-newer`, `display-left-newer-orphans`, `display-right-newer-orphans`, `display-left-orphans`, `display-right-orphans`, and `all`, `mismatches`, `matches`, `none`), plus `display-context` = differences with `CompareStructure` | both | text: side-by-side, interleaved, XML; folder: list, side-by-side, XML, JSON | defaults |
| ignore | `ignore-unimportant` | both | all | off |
| numbers | `line-numbers`, `no-line-numbers` | text | side-by-side (text), interleaved | `defaults.line_numbers` |
| strike | `strikeout-left-diffs`, `strikeout-right-diffs` (both may be given) | text | interleaved | none |
| patch | `patch-normal`, `patch-context`, `patch-unified` | text | patch | unified |
| columns | `column-size`, `column-timestamp` (`-modified`, `-mtime`), `column-attributes`, `column-crc`, `column-owner`, `column-group`, `column-status`, `column-none` (alone); `column-vcs`, `column-revision`, `column-version` → `unsupported` | folder | side-by-side | size, timestamp |
| links | `include-file-links` | folder | side-by-side HTML | off |
| palette | `html-color` (`html-colour`), `html-mono`, `html-custom:<css>` (colour plus the stylesheet), `text` (plain; ANSI comes from the terminal and `--no-color`, as M1) | both | every layout with an HTML form | plain text |
| print palette | `print-color`, `print-mono` | both | HTML layouts (print block); the printer target | colour |
| orientation | `print-portrait`, `print-landscape` | both | HTML layouts (print block); the printer target | none; portrait for the printer |
| wrap | `wrap-none`, `wrap-character`, `wrap-word` | text | side-by-side, interleaved (text and HTML) | character |

- **Conflicts.** Two different words of one slot → `Conflict` (exit 100), whatever their order, also when one came from an
  M1 flag (`--html-mono` with `output-options:html-color`). The same word twice is one. Columns accumulate; `column-none`
  with another column is a conflict. `context` lines and `width` are numbers, not words: they come from `defaults`
  (`--context`, `--width`, options).
- **Wrong kind, unknown.** A word of the other kind (`column-crc` in a text report) or no word at all → `InvalidOption`
  naming it, exit 100. **Unused** (a word of this kind whose slot the layout does not use: `strikeout-left-diffs` with
  side-by-side) → kept in `unused`, one warning `report_option_unused` "<layout> does not use: <words>". The dialog's
  remembered words span layouts (D-208), and a script line copied to another layout still runs. The palette is the
  exception: an HTML palette word with a layout that has no HTML form is `unsupported`, as in M1.
- **Before the comparison.** `parse` and `check_target` run before anything is compared, so a refused combination exits
  100 with nothing compared or written. This revises M1 §1.9.3 item 10a (refused after the comparison); the exit code is
  the same.
- **`ignore-unimportant`.** The word sets the comparison's ignore-unimportant too, as the compare subcommands' flag
  does: `report_request` runs before the comparison and the adapter passes `ReportRequest::ignore_unimportant()` into
  the comparison's request. M1's rule that a report follows the comparison's effective flag (wave-4 review) stays, so
  the report, the verdict and the exit code agree.
- **Canonical words.** `words()` gives the layout word, then the option words in the table's slot order, each in its first
  spelling above, then the output words; `parse(words())` equals the request (property P-RPTREQ, A7.2).

**Layout × format (normative).** HTML is `html-color`, `html-mono` or `html-custom`; ANSI only on a terminal's stdout
(as M1: never to a file, the clipboard or a printer).

| Kind | Layout | Text | ANSI | HTML | Other |
|---|---|---|---|---|---|
| text | `unified` (annotated) | yes | yes | — | |
| text | `side-by-side` | yes | yes | yes | |
| text | `summary` | yes | yes | yes | |
| text | `interleaved` | yes | yes | yes | |
| text | `patch` | yes | yes | — | |
| text | `statistics` | CSV | — | — | |
| text | `xml` | — | — | — | XML only (a palette word → `unsupported`) |
| folder | `list` | yes | yes | — | |
| folder | `side-by-side` | yes | yes | yes (+ links) | |
| folder | `summary` | yes | yes | yes | |
| folder | `json` | — | — | — | JSON only |
| folder | `xml` | — | — | — | XML only |

**Targets (normative).** Stdout and a file take every layout. The clipboard takes every layout except one with links.
`printer` and `browser` take the layouts with an HTML form; their palette is the print palette, so `html-color` and
`html-mono` are unused there. Links need folder side-by-side HTML and a target that is a local file, `printer` or
`browser` (`unsupported` otherwise: "file reports need a folder on disk"). A scope (§ below) needs a row layout.

**Scope (RPT-003 *Just selection*, D-209).** `ReportScope` is set by the Service, never by words:

- `TextLines { side, first, last }` (`--lines [left:|right:]<a>-<b>`, left by default; `parse_line_range`): the rows from
  the first row whose line on `side` is ≥ `first` to the last whose line on `side` is ≤ `last`, other rows between them
  included. Nothing in range → the header and `0 rows`.
- `FolderRows(selection)` (`--select <MASK>…`, `--path <REL>…`, W6's words through `select::select` and the `--path`
  rules of §1.6.17, every folder expanded): the selected rows, everything below a selected folder, and their ancestors
  for the structure; the display filter applies on top.
- Row layouts only: text side-by-side, interleaved, XML; folder list, side-by-side, XML. Summary, statistics, patch and
  JSON with a scope → `invalid_input` (counts or a patch of part of a comparison would mislead). The header's `Mode:`
  line ends `; scope: lines <a>–<b> of the left` or `; scope: <n> selected rows`; XML `mode/@scope`.

**HTML wrap and the print block (D-210).** In the HTML of text side-by-side and interleaved, `wrap-character` is M1's
rule (`white-space: pre-wrap; overflow-wrap: anywhere`), `wrap-word` is `pre-wrap` with `overflow-wrap: break-word`,
`wrap-none` is `white-space: pre` (the page scrolls). A print block is appended to the stylesheet when an orientation
word is given or the target is `printer`: `@page { size: <orientation>; margin: 12mm }` and `@media print { … }` with
`print-color-adjust: exact`, repeated table headers, rows not split, and, when the print palette differs from the
screen palette, that palette's rules. Without these words every HTML byte is M1's (A7.8).

#### 1.7.5 Interleaved layout (D-197)

One column of lines in row order. A same row: one line, both numbers. A changed row: its left line, then its right
line. A left-only row: its left line; a right-only row: its right line. A run of hidden rows (display modes, as RPT-010):
one row `… N lines hidden …`. Pieces of a line broken by the characters-per-line limit keep the line's number, as in
side-by-side.

**Text form (normative).** `<left number> <right number> <mark> <text>`, numbers right-aligned to the widest number of
the comparison, blank when absent; without `line-numbers` the two number columns are left out. `<mark>` has two
characters: the side (` ` both, `-` the left line of a difference, `+` the right line) and the importance (` ` same,
`!` important, `~` unimportant; ` ` too under `ignore-unimportant` for an unimportant difference, whose two lines are
still both shown). The text column wraps by `wrap-*` in `width − gutter` columns with the side-by-side functions (tabs,
display widths, escapes of controls and bidirectional formatting characters: unchanged rules of docs/13). ANSI: marks and
differing text red (important) or blue (unimportant).

**HTML form.** One self-contained document as side-by-side HTML (header, legend, embedded stylesheet, no scripts, no
generated ids): a table `il` with columns left number, right number, text; rows classed `imp`/`unimp` and `l`/`r` (the
line's side), orphans `lo`/`ro`, hidden runs `gap`; inline differences `<span class="imp|unimp">`.

**Strike-out.** `strikeout-left-diffs` strikes the left side's differences: the inline differences of its changed lines
and the whole text of its left-only lines; `strikeout-right-diffs` the same for the right. HTML: `<del class="imp|unimp">`
(`text-decoration: line-through` in every palette, so mono keeps it); ANSI: SGR 9 with the colour; plain text: the
struck span written `[-…-]` (a literal `[-` or `-]` in the content is left as it is: the mark is for reading, not for
parsing). Without strike-out, plain text marks no inline span, as side-by-side text.

#### 1.7.6 XML dumps (D-198)

One document type, `hikasa-report` in the namespace `urn:hikasa:report:1`, for text and folder comparisons. The XSD is
`docs/schemas/hikasa-report-1.xsd`, written by hand and normative; docs/13 shows the table below and links it.

**Bytes.** `<?xml version="1.0" encoding="UTF-8"?>` then `\n`; UTF-8 without BOM; `\n` line ends; two spaces per depth;
one element per line, a text `row` with its children on one line; attributes in the table's order; booleans `true`,
`false`; times RFC 3339, UTC for file times (as the side-by-side columns), the report time as D-206. Escaping: `&`, `<`,
`>` always, `"` in attributes; a CR is `&#13;` and a TAB in an attribute `&#9;` (XML parsers normalize them away
otherwise); a character XML 1.0 cannot carry (C0 controls except TAB and LF, U+FFFE, U+FFFF) is the empty element
`<c x="1b"/>` (lower-case hex code point) in text content. In an attribute, where elements cannot go, a name is written
as the folder list writes it (`escape_name`: such characters as `\u{…}`, a backslash as `\\`), and `raw-name` then
holds its exact bytes.
Bidirectional formatting characters are data and stay. Display modes, `ignore-unimportant` and the scope apply; colours,
wrap, width and columns do not (a dump has every column).

| Element | Parent | Attributes (required in **bold**) | Content |
|---|---|---|---|
| `hikasa-report` | — | **`version`** = `1`, **`type`** = `text`\|`folder` | `header`, then `rows` (text) or `tree` (folder), then `summary` |
| `header` | root | — | `title`, `generated`?, `side` × 2, `mode`, `rules`, `error`*, `status`?, `warning`* |
| `title` | header | — | the title |
| `generated` | header | **`time`**, **`by`** (`hikasa <version>`) | empty; absent without a report time (D-24) |
| `side` | header | **`id`** = `left`\|`right`, **`spec`**; text only: **`encoding`**, **`bom`**, **`eol`** (`lf`, `cr_lf`, `cr`, `mixed`, `none`), **`binary`** | empty |
| `mode` | header | **`display`** (the canonical word without `display-`), `context` (with `display-context`), **`ignore-unimportant`**; folder: **`folders`** (`always`, `structure`, `files-only`), **`suppress-filters`**; `scope` | empty |
| `rules` | header | — | the RPT-004 `Rules:` text |
| `error` | header | **`side`** | the side's load error |
| `status` | header | **`complete`** = `false` | why (a cancelled comparison) |
| `warning` | header | **`code`** | the message; the comparison's and the rendering's warnings |
| `rows` | root (text) | — | (`row` \| `hidden`)* |
| `row` | rows | **`state`** = `same`\|`changed`\|`left-only`\|`right-only`; `importance` = `important`\|`unimportant` (not on `same`); `left`, `right` (line numbers, absent on a missing side); `left-piece`, `right-piece` (2, 3, … when that side's line in the row is a piece after an artificial break; each side apart, D-213) | `l`?, `r`? (a same row has both) |
| `l`, `r` | row | — | mixed: text, `d`, `u`, `c` |
| `d`, `u` | l, r | — | mixed: text, `c`; an important (`d`) or unimportant (`u`) inline difference |
| `c` | l, r, d, u; title, rules, error, status, warning (W7a note 6) | **`x`** | empty |
| `hidden` | rows | **`rows`** | empty |
| `tree` | root (folder) | `flat` = `true` in flatten mode | (`folder` \| `entry`)* |
| `folder`, `entry` | tree, folder | **`name`**, **`path`** (D-29's), **`state`** = `same`\|`left-newer`\|`right-newer`\|`different`\|`unimportant`\|`left-only`\|`right-only`\|`unknown`\|`error`\|`filtered`, **`marker`** (the list's character); `entry`: **`kind`** = `file`\|`link`\|`other`; `ignored` = `true` | `left`?, `right`?, `file-report`? (entry); then, in a `folder`, its children |
| `left`, `right` | folder, entry | **`name`**; `raw-name` (lower-case hex of the OS bytes, when `name` is lossy or escaped); `size`, `modified`, `attributes`, `crc` (8 hex digits), `owner`, `group`, `link-target`, `error`, each when known | empty |
| `file-report` | entry | **`status`** = `written`\|`skipped`\|`failed`; `href` (written); `reason` (otherwise) | empty |
| `summary` | root | **`verdict`** (as `hikasa.*_compare.v1`); folder: **`complete`** | `count`* |
| `count` | summary | **`group`**, **`name`**, **`value`** | empty |

`summary/count` lists the counts of the summary layout in its order (text: rows, sections and lines by importance;
folders: file and folder pairs per state, files, folders and bytes per side, content-test results). One function,
`summary_counts`, feeds the text and HTML summaries and the XML (T11), so the three cannot disagree. A `rows` element of
a comparison with no row is empty; `tree` likewise. Folder elements are nested because XPath and XSLT walk trees; the
JSON report stays flat for `jq` (D-29).

**Versioning.** `version="1"` and the namespace name the major version. Within version 1, attributes and elements are
only added, optional, with the XSD updated in the same change; a consumer ignores what it does not know. A removal or a
change of meaning is version 2 with the namespace `urn:hikasa:report:2`. Both rules are in docs/13.

#### 1.7.7 XML validation (D-199)

- **Well-formedness and structure, everywhere.** `core/hikasa-report/tests/xml_schema.rs` parses every XML golden and
  every document P-XML generates with `roxmltree` (dev-dependency, §1.7.16) and checks: the namespace, `version`, the
  element of each row in the table (parent, required attributes, enumerations), no character XML 1.0 forbids, and the
  round trips of A7.5 and A7.6.
- **Against the XSD, on Linux and macOS CI.** The same test runs `xmllint --noout --schema
  docs/schemas/hikasa-report-1.xsd` on the goldens and on 64 generated documents. `HIKASA_REQUIRE_XMLLINT=1` makes a
  missing `xmllint` a failure; `ci.yml` sets it for the Linux and macOS test jobs, and the Linux job installs
  `libxml2-utils` first (macOS ships `xmllint` with its libxml2). The maintainer approved the CI package, the variable and
  `roxmltree` as a test-only dependency (standing order after the first text). On Windows the check is skipped with one line on stderr: the bytes are
  the same on every platform (INV-RPT-1), which the goldens prove there.
- **The check can fail.** A negative case feeds `xmllint` a golden with one required attribute removed and one unknown
  attribute added and expects both rejections; the XSD itself is validated by `xmllint --noout` against the XML Schema
  namespace on load, so a broken XSD fails every case.

#### 1.7.8 Folder report with links to file reports (D-200)

**Which pairs.** With `include-file-links`, every row the folder report shows that is a file pair (`kind` file, both
sides present, links followed by FC-013 count as files) whose category under the report's `ignore-unimportant` is
`LeftNewer`, `RightNewer` or `Different` (also `~`) gets a text report. Orphans, folders, links shown as links, `same`,
`unknown`, `error` and `filtered` rows get none. The numbering is fixed by the folder report alone: the `n`-th such row
in report order, 1-based.

**The pair's comparison and report.** `Service::pair_text_request(&FolderCompareRequest, &PreparedFolderCompare,
NodeId) -> Option<TextCompareRequest>` is the one builder of a pair's Text Compare request (T11): both sides' exact specs
(`TextCompareRequest::for_specs`), the folder request's text rules file and patch, the prepared format overrides
(FF-002) and the effective `ignore_unimportant`. The GUI's `FolderViewModel::open_request` calls it instead of its own
code (no behaviour change; its tests stay). A pair report is text side-by-side HTML (`FileLinks::text`; the API may pick
interleaved) with display all, line numbers, `wrap-character`, the folder report's palette, stylesheet and print block,
and a header line `Folder report: <link>` before `Left:`. A pair over the size limit of rules-based content comparison
(warning `rules_size_limit`, docs/04) is `skipped` (`too_large`) without being read; a side found binary when compared
is `skipped` (`binary`); a comparison or render that fails is `failed` with its error and the warning
`file_report_failed` (the folder report is still written, the exit code stays the comparison's). Pair comparisons run
on the service pool through the scan's content budget (`ByteBudget`, D-174), each compared, rendered, written and
dropped before its budget is released.

**Names (normative).** `safe_component(name, max)` (pure, `hikasa_session::api::report_write`): every char that is not
a letter or a digit (Unicode) or one of `.`, `-`, `_` becomes `_`, also a lossy U+FFFD; a leading `.` or `-` becomes
`_`; a name `hikasa_vfs::archive::name::windows_name_problem` (§1.6.18) refuses (device names, also before an
extension; a trailing dot) gets a leading `_`; truncated at a char boundary to `max` bytes; empty → `file`. Then:

| What | Name |
|---|---|
| the main file | as given (`--output-to`); its stem is its final name without its last extension (`report` for `report.html`, `report` for `.html`) |
| the files folder | `<safe_component(stem, 64)>_files`, next to the main file; `-2`, `-3`, … when that is not ours (§1.7.9) |
| a pair's report | `<n>-<safe_component(left final name, 64)>.html`, `n` zero-padded to the digits of the number of linked rows |
| the marker | `.hikasa-report.toml` in the files folder |

The numbers make the names distinct on any file system, case-insensitive ones included, whatever the pairs are called;
safe names read well and travel to Windows.

**Hrefs (normative).** `hikasa_report::links::href(segments)`: each segment's bytes, RFC 3986 unreserved bytes (`A–Z
a–z 0–9 - . _ ~`) kept, every other byte `%XX` (upper-case hex), segments joined by `/`. Folder report → pair report:
`<folder>/<file>`, both safe components. Pair report → folder report: `../<main final name>`, from the OS bytes on Unix
(a name that is not UTF-8 still links) and from the name on Windows when it is valid Unicode; otherwise the line shows
the name without a link. Links are never absolute, never `file:`, never with `\`, `\\?\` or a drive letter, whatever
spelling the target was given in.

**Rendering.** `render_report` with `links: Some(..)` (§1.7.4): in side-by-side HTML both side names of a linked row
are `<a href="…">`; a skipped or failed row's names carry `title="no file report: <reason>"`. `PairLinks::link(node) -> Option<PairLink>` with
`PairLink::{Href(String), Skipped(reason), Failed(message)}` is answered by the plan, so the renderer neither names nor
writes files. XML: `entry/file-report`. Text output and other layouts never link.

#### 1.7.9 Writing reports: targets, plans, streaming (D-201, D-202)

```rust
// hikasa_session::api::report (additive)
pub enum ReportTarget { Stdout, File(OsString), Clipboard, Printer, Browser }
impl ReportTarget {
    /// `-` → Stdout; `clipboard` or `clipboard://` → Clipboard; `printer`, `browser`; else a file spec (`./printer` is a file).
    pub fn parse(arg: &OsStr) -> ReportTarget;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportPlan {
    pub kind: ReportKind, pub words: ReportWords, pub target: TargetDto,
    pub main: Option<Spec>,               // the file written (File, Printer, Browser)
    pub replaces: bool,                   // the main file exists
    pub links: Option<LinksPlan>,
    pub warnings: Vec<Warning>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinksPlan {
    pub folder: Spec, pub wanted: Spec,   // `folder != wanted`: renamed, warning `report_folder_renamed`
    pub state: FilesFolder,
    pub pairs: Vec<PairPlan>,             // report order
    pub stamps: Vec<(String, ItemStamp)>, // the old folder's listing (ReplaceOurs), checked again at execution
}
pub enum FilesFolder { New, ReplaceOurs }                   // snake_case in JSON
pub struct PairPlan { pub node: NodeId, pub path: String, pub file: String, pub left: Spec, pub right: Spec, pub skip: Option<String> }
pub struct PairOutcome { pub path: String, pub file: Option<String>, pub status: PairStatus, pub reason: Option<String>, pub error: Option<ErrorReport> }
pub struct ReportOutcome {
    pub plan: ReportPlan, pub dry_run: bool, pub cancelled: bool, pub bytes: u64,
    pub pairs: Vec<PairOutcome>, pub clipboard: Option<ClipboardPayload>, pub opened: Option<PathBuf>, pub warnings: Vec<Warning>,
}
impl ReportOutcome { pub fn dto(&self) -> ReportDto; }   // hikasa.report.v1
```

**Without links**, a plan is the target and nothing more: `main` resolved through `write_spec` (as `--output`, an archive
member allowed, its folder must exist), `replaces` from one `lstat`. **With links** it is built from the model, one
`lstat` of the main file and one listing of each files folder it considers, the wanted one first:

1. The wanted folder `<safe stem>_files` is absent → `New`.
2. It is a folder holding `.hikasa-report.toml` that parses (`schema_version = 1`, T13), whose `report` equals the main
   file's final name (compared as the back-end compares names), and the folder holds exactly the marker and the files it
   lists, each with the listed size and modification time → **ours**: `ReplaceOurs`, with the listing as `stamps`.
3. Anything else (a file, a folder without a marker or with another `report`, an extra file, a changed file, a listing
   that fails) → the first of `<…>_files-2` … `<…>_files-99` that is absent (`New`) or ours (`ReplaceOurs`), with the
   warning `report_folder_renamed` naming both; none → `already_exists`, exit 100.

Nothing that is not ours is ever written into, replaced or removed. The marker:

```toml
schema_version = 1
report = "report.html"          # the main file's final name; `report_raw` (hex of the OS bytes) when not UTF-8
generator = "hikasa 0.1.93"
[[file]]
name = "001-main.c.html"
size = 18342
modified = 1793000000123456789  # ns since the epoch, as written
```

**Execution (normative order).**

| Step | Action | On failure or cancel |
|---|---|---|
| 1 | create the staging folder, a free hidden sibling `.<folder>.hikasa-tmp[-n]` | nothing to undo |
| 2 | per pair: compare, render into `staging/<file>` (`open_write`, `Durability::None`); then the marker | remove staging |
| 3 | render the main report into its atomic writer (temporary sibling) | remove staging; drop the writer (its temporary file goes) |
| 4 | `ReplaceOurs`: list the old folder again; it must equal `stamps`; rename it to a free `.<folder>.hikasa-old[-n]` | a difference fails the report "changed since the plan was made"; remove staging, drop the writer |
| 5 | rename staging to `<folder>` | step 4's rename back; as above |
| 6 | commit the main writer (`FileOnly` when it replaces, `None` when new: D-164's cases) | rename `<folder>` back to staging and remove it; step 4's rename back |
| 7 | remove the old folder (only the marker and the listed files, then the empty folder) | warning `report_old_folder_kept` naming it; the report stands |

Cancellation is checked before each step up to 4 and between pairs; from step 5 the report runs to its end. A crash
between steps leaves at most the hidden staging or old folder, never a half-written `<folder>` or main file; the next run
picks other hidden names and does not touch them. A plan of a printer or browser target is always `New` in a fresh
folder (§1.7.11).

**Streaming (D-202).** `Service::write_output_with(target, job, render: &mut dyn FnMut(&mut dyn Write) ->
Result<(), ApiError>) -> Result<(Spec, u64), ApiError>` opens the VFS's atomic writer and lets the renderer write into
it through a `BufWriter` of 64 KiB; nothing holds the report. A render error or a cancel drops the writer, so the
target is untouched (M1 `--output` semantics, also for a link target, a FIFO and a device as docs/12 says).
`write_output(target, bytes)` stays as a wrapper. Cancellation reaches the pure renderers through `CancelWriter`, which
checks the token every MiB written and returns an error the Service maps to `Cancelled`; the renderers stay without a
`Ctl`. Stdout streams the same way. Only the clipboard holds a report, up to `CLIPBOARD_REPORT_MAX` = 128 MiB (text and
HTML flavours together); beyond it rendering stops at once and the report fails `too_large`, nothing placed.

**`--dry-run`.** The comparison runs; pair comparisons and every write do not. Human lines: `write <main>` or `replace
<main>`; with links `mkdir <folder>`, `replace <folder> (an earlier report)` or `mkdir <folder> (instead of <wanted>,
which is not this report's)`, then `write <folder>/<file>  <path>` per pair (`skip (<reason>) <path>` for one already
known to be skipped), then `would write <n> files`. JSON: `hikasa.report.v1` with `dry_run: true`. A dry run's plan is
the plan execution takes (T12; A7.12).

#### 1.7.10 Clipboard (D-203)

```rust
// hikasa_session::clipboard (additive)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload { pub text: String, pub html: Option<String> }
pub trait ClipboardProvider: Send + Sync {
    fn read(&self) -> Result<ClipboardContent, ClipboardError>;
    /// Places a report. Default: `Unavailable`. Called by the adapters only (architecture check 2).
    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> { /* Unavailable */ }
}
```

- **Payload.** A layout without an HTML form: `text` = the report's bytes. An HTML report: `html` = the report's bytes,
  `text` = the same request rendered as plain text (same layout, palette `text`), so a plain editor pastes a readable
  report; XML, CSV, JSON, patches: `text` only. Bytes as the file target's (`\n` line ends on every platform).
- **Providers.** `SystemClipboard`: arboard `set_text`, or `set_html(html, Some(text))`. **On Linux (X11 and Wayland) the
  content outlives `hikasa`** (maintainer's order, after the first text): `write_payload` hands the payload to a small
  helper process, `hikasa` itself started through a hidden internal entry (not in `--help`, the reference or the help
  topics), which reads the payload from its stdin, places it with arboard's Linux wait mode (`SetExtLinux::wait`), owns
  the selection until another program takes it, then exits, as the usual command-line clipboard tools do. The CLI
  returns once the helper reports the payload placed (one status line on its stdout) and does not wait for it to end;
  a helper that cannot start or reports a failure is `Failed` (`io`), nothing placed. No new runtime dependency: the
  helper uses the arboard the clipboard adapter already links; it is spawned only by the adapter (`host.rs`), never by
  core. Windows and macOS keep the content after the process ends without a helper. `FileClipboard` (`HIKASA_CLIPBOARD_FILE`): writes `html` when present, else
  `text`, atomically, so a CLI test reads back exactly a file target's bytes. `FixedClipboard` records every payload.
  `NoClipboard`: `Unavailable`. Help `report` says that on Linux a helper keeps the content until something else is
  copied.
- **Who writes.** `report_execute` returns the payload in `ReportOutcome::clipboard`; the CLI calls `write_payload` on the
  provider it gave `ServiceInit`; the GUI (W13) writes its own clipboard; the script interpreter (W8) hands the payload
  to its adapter. Errors map as §1.3.6 (`Unavailable` → `unsupported`, `Failed` → `io`), exit 100.

#### 1.7.11 Printer and browser (D-204)

`printer` and `browser` write the report into a new folder `<cache>/print/<yyyymmdd>-<hhmmss>-<n>/` (`n` makes it free;
the time is the report time of D-206) as `<safe_component(title or "report", 64)>.html`, with its
files folder when linked, then call `AppLauncher::open_associated` on the main file (D-186: tests use
`RecordingLauncher`, never a program). `printer` adds the print block (orientation from `print-*`, portrait by default)
and uses the print palette; `browser` uses the print palette without the print block. The bytes are those of a file
target given the words the target implies (`html-<palette>`, `print-<palette>`, `print-<orientation>` for `printer`),
which A7.17 compares. A launcher that fails or is `NoLauncher` → exit 100 "no application could open <path>"; the file
stays. Before writing, folders of that pattern below `<cache>/print/` older than seven days by the injected clock are
removed, best effort (a failure is a debug log line). Native printing stays out (D-62); W13's *Print Preview* is
`printer` and *View in Browser* is `browser`.

#### 1.7.12 Report time and identical output (D-206, D-207)

`Service::report_time(&self) -> Option<jiff::Zoned>`: `EnvFacts::source_date_epoch` (captured from `SOURCE_DATE_EPOCH`
by `capture_env`: a decimal integer of seconds, shown in UTC), else `self.clock().now()` in the user's zone. A value
that does not parse, or lies outside jiff's range, is ignored with one warning `source_date_epoch_invalid`.
One report job takes it once: the folder report and all its pair reports carry the same `Generated:` time. Only reports
read it (not `touch`, not the log).

RPT-011 holds by construction and is proven by bytes: for one comparison and one request, the CLI's `report`, the
compare subcommand's shortcut, `Service::report_execute` into a `Vec`, a file, stdout, the clipboard payload and the
printer file are byte-identical (the printer and browser files with the words of §1.7.11). The tests fix the time with
`SOURCE_DATE_EPOCH` on both sides (A7.17). The GUI renders through the same calls (W13).

#### 1.7.13 Remembered choices (D-208)

`state.toml` `[dialogs]` (§1.1.4, "free tables per dialog id") gets two tables owned by the report model:

```toml
[dialogs."report-text"]          # and [dialogs."report-folder"]
layout = "interleaved"
options = ["display-context", "line-numbers", "strikeout-left-diffs"]
output_options = ["html-color", "wrap-word"]
output_to = "file"                # file, clipboard, printer, browser
file = "/home/u/reports/a.html"   # the last file target, as typed
```

`Service::report_choices(kind) -> ReportChoices` reads them leniently: a word `ReportRequest::parse` refuses is dropped
with one warning `invalid_config_value`; a missing table gives the defaults. `Service::remember_report_choices(kind,
&ReportChoices)` writes through `state_update` (D-108). The title is not remembered (it names one comparison). The CLI
never reads or writes them: a command line means the same whatever a dialog did last (A7.19). W13's dialog and W8's
scripts are the users; `state.toml`'s schema stays version 1 (an additive table of a free section).

#### 1.7.14 Service

```rust
// hikasa_session::api::report (additive)
pub struct ReportFlags {                           // the CLI's flags, raw; core turns them into words (T11)
    pub words: ReportWords,                        // --layout/--report, --options, --output-options, --title
    pub html: Option<bool /* mono */>, pub wrap: Option<String>, pub patch_style: Option<String>,
    pub columns: Option<String>, pub line_numbers: Option<bool>, pub display: Option<String>, pub folders: Option<String>,
    pub suppress_filters: bool, pub context: Option<u32>, pub width: Option<u16>,
    pub lines: Option<String>, pub select: Option<SelectRequest>,     // scope
}
pub enum ReportSource<'a> {
    Text(&'a TextCompareResponse),
    Folder { request: &'a FolderCompareRequest, prepared: &'a PreparedFolderCompare, response: &'a FolderCompareResponse },
}
impl Service {
    /// Flags → words (appended after `flags.words`), defaults of `default_layout`, parse, `check_target`. Before the comparison.
    pub fn report_request(&self, kind: ReportKind, default_layout: &'static str, flags: &ReportFlags, target: &ReportTarget)
        -> Result<ReportRequest, ApiError>;
    pub fn report_plan(&self, source: ReportSource<'_>, req: &mut ReportRequest, target: &ReportTarget, job: &JobControl)
        -> Result<ReportPlan, ApiError>;          // sets the scope; reads `html-custom`'s stylesheet through the VFS
    pub fn report_dry_run(&self, plan: ReportPlan) -> ReportOutcome;
    pub fn report_execute(&self, source: ReportSource<'_>, req: &ReportRequest, plan: ReportPlan,
        stdout: Option<&mut dyn Write>, job: &JobControl) -> Result<ReportOutcome, ApiError>;
    pub fn pair_text_request(&self, req: &FolderCompareRequest, prepared: &PreparedFolderCompare, node: NodeId)
        -> Option<TextCompareRequest>;
    pub fn report_time(&self) -> Option<jiff::Zoned>;
    pub fn report_choices(&self, kind: ReportKind) -> ReportChoices;
    pub fn remember_report_choices(&self, kind: ReportKind, choices: &ReportChoices) -> Result<(), ApiError>;
    pub fn write_output_with(&self, target: &OsStr, job: &JobControl,
        render: &mut dyn FnMut(&mut dyn Write) -> Result<(), ApiError>) -> Result<(Spec, u64), ApiError>;
}
```

Each runs through `run_guarded`. `render_text_with`, `render_folder_with` stay and become `report_execute` with a
`Stdout` target and no plan. The text and folder comparisons run before `report_plan` as today (`text_compare`;
`folder_prepare_in` + `folder_run`, which keeps the prepared value the pair requests need). `JobKind::Report` (exists)
covers the pair comparisons and the writes, `Phase::Writing` while files are written. `ApiOp::Report { kind }` (new).
`WarningCode` gains `report_option_unused`, `report_folder_renamed`, `report_old_folder_kept`, `file_report_failed`,
`source_date_epoch_invalid`. A stylesheet of `html-custom:<css>` is read through the VFS, at most 1 MiB (`too_large`);
`-` and `clipboard://` are refused for it.

#### 1.7.15 CLI and JSON (D-205)

```
hikasa report text   [<LEFT> <RIGHT>] [--session <REF>] <rule flags of text compare>   REPORT
hikasa report folder [<LEFT> <RIGHT>] [--session <REF>] <rule flags of folder compare> REPORT
REPORT = [--layout|--report <WORD>] [--options <WORD,…>]… [--output-options <WORD,…>]… [--output-to <TARGET>] [--output <FILE>]
         [--title <TEXT>] [--display <WORD>] [--folders <MODE>] [--suppress-filters] [--context <N>]
         [--patch-style <S>] [--columns <LIST>] [--html|--html-mono] [--line-numbers|--no-line-numbers] [--width <N>]
         [--wrap <MODE>] [--lines [left:|right:]<A>-<B>] [--select <MASK>…] [--path <REL>…] [--dry-run]
```

- **One group.** `#[command(flatten)] ReportArgs` holds REPORT and is used by `text compare`, `folder compare`, `session
  run`, `report text` and `report folder` (one owner, like `FolderRuleArgs`); the flags that only one kind takes are
  refused for the other by `report_request` (`invalid_input`), not by clap, so the five commands parse alike. A clap
  reflection test checks the five share it. The rule flags are the compare subcommands' own groups.
- **Defaults.** `report` defaults to `side-by-side`; every other default is the compare subcommand's (M1 docs/12 §4), so
  `hikasa report text a b <flags>` and `hikasa text compare a b --report side-by-side <flags>` print the same bytes and
  exit alike (A7.18). `--output <FILE>` is `--output-to <FILE>`; both → usage error, exit 100.
- **Targets.** `--output-to -` (stdout, the default), `clipboard` (or `clipboard://`), `printer`, `browser`, or a file
  spec; a file named like a keyword is written `./clipboard`. Stdout is the only target that prints the report; the
  others print nothing on stdout, as M1's `--output`.
- **Specs.** `<LEFT> <RIGHT>` or `--session <REF>` (a text-compare session for `text`; a folder-compare or folder-sync
  session for `folder`), with flags on top, as `session run`. `-` and `clipboard://` as sources, as `text compare`.
- **`--json`.** `hikasa.report.v1`; with the stdout target → usage error, exit 100 ("the report and the JSON document
  cannot share stdout: pass --output-to"). The compare subcommands keep their own document with `--json` (M1), the
  report going to its target.
- **Exit codes.** The comparison's (M1 §2.21), as the compare subcommand: a report is a way to show a result, not a
  result of its own. 100 for a refused word, combination or target, a write that fails (target untouched), a stylesheet
  that cannot be read, `too_large`, a launcher that fails, a cancel; 107 where the compare subcommand gives it. A pair
  report that fails is a warning (§1.7.8).
- **Human output.** Nothing on stdout but a stdout report or a dry run's lines (§1.7.9); warnings on stderr as usual.

**JSON.** Appended to `SCHEMAS`, 38 → 39:

| Schema | `result` |
|---|---|
| `hikasa.report.v1` | `{type: "text"\|"folder", layout, format: "text"\|"html"\|"xml"\|"csv"\|"json", target: "file"\|"clipboard"\|"printer"\|"browser", output: str\|null, replaced: bool, dry_run, cancelled, bytes, verdict, words: {layout, options, output_options}, unused: [str], links: null\|{folder, wanted, renamed: bool, state: "new"\|"replace_ours", pairs: [{path, file: str\|null, status: "planned"\|"written"\|"skipped"\|"failed", reason: str\|null, error: ErrorReport\|null}], counts: {written, skipped, failed}}, clipboard: null\|{text_bytes, html_bytes}, opened: str\|null}` |

`verdict` is the comparison's, as in `hikasa.text_compare.v1` and `hikasa.folder_compare.v1`. Paths through
`write_json` (§1.5.13).

#### 1.7.16 Dependencies and architecture test (D-211)

- **No runtime crate.** `roxmltree` 0.21.2 (MIT OR Apache-2.0; its only dependency `memchr` is in tree) becomes a
  dev-dependency of `hikasa-report` at the version `Cargo.lock` already holds through `fontique`: `cargo tree` and
  `Cargo.lock` gain no package. Its `docs/02` line says *dev-only*. `cargo deny` passes without new exceptions.
  `xmllint` (libxml2) is a CI tool, installed in the Linux job, never linked.
- **Architecture test.** Check 3: `hikasa-report`'s dev list += `roxmltree`. Check 2: the token `write_payload` only in
  `core/hikasa-session/src/clipboard.rs`, `host.rs`, `cli/`, `gui/` and tests; `report_write.rs` names no
  `std::fs`. Check 7: the adapters call none of the report slot parsers (`TextLayout::parse`, `FolderLayout::parse`,
  `OutputFormat::parse`, `Wrap::parse`, `PatchStyle::parse`, `DisplayMode::parse`, `FolderColumns::parse_list`,
  `parse_folder_mode`), and `ReportRequest::parse` is called only in `hikasa_session::api::report` (T11). No `unsafe`.

#### 1.7.17 Gate G13 and budgets (D-212)

Measured at the end of W7b on an idle machine, release build, inputs generated by the test as G1 and G2's; recorded in
docs/12 beside G1 and G2. The harness is `cli/hikasa-cli/tests/perf_gates.rs` (`#[ignore]`): a warm-up, then the median
of three, peak RSS from the child's `ru_maxrss`, each new run against `--report summary` of the same input in the same
run.

| Item | Measure | Where | Threshold |
|---|---|---|---|
| G13 text | `report text` of the G1 pair to a file, display all: side-by-side HTML, interleaved HTML, interleaved text, XML | `perf_gates.rs` | each ≤ the summary run + 1.5 s; peak RSS ≤ the summary run's + 24 MiB |
| G13 folder | `report folder` of the G2 tree (warm), display all: side-by-side HTML, XML | `perf_gates.rs` | each ≤ the summary run + 1.0 s; RSS ≤ + 32 MiB |
| G13 links | the G2 tree with 1,000 differing pairs of 4 KiB files, `include-file-links`, a new files folder | `perf_gates.rs` | ≤ 5 s end to end; RSS ≤ the summary run's + 64 MiB |
| Streaming | a counting allocator: the peak of live heap while rendering 50,000 and 200,000 synthetic text rows in each new text layout into a sink; 25,000 and 100,000 folder rows in folder side-by-side HTML and XML | `core/hikasa-report/tests/stream_memory.rs` (CI) | text: ≤ 4 MiB, and the two sizes within 1 MiB of each other; folder: ≤ 4 MiB + 32 B per visible row |
| Cancel | a cancel at the n-th MiB of a file output: at most one MiB more is written, the target and the folder are as before, no temporary or hidden folder is left | `report_write.rs` tests on `MemoryVfs` | G4 by construction |
| Clipboard | a payload over 128 MiB fails `too_large` holding at most 128 MiB + 64 KiB | unit test | listed |

The streaming row runs on CI and is the property that matters; the gate's times have room (G1's comparison is 0.43 s of
its 3 s). A failure is reported with the figures, as a failed gate (docs/17 §7.1), not met by relaxing a rule. G1, G2
and G4 are unchanged.

#### 1.7.18 Hooks for later waves

- **W8.** `text-report`, `folder-report` map to `ReportWords` (the line's `layout:`, `options:`, `output-options:`,
  `title:` as written, after `%`-expansion) and `ReportTarget::parse(output-to:)`; the interpreter calls
  `report_request`, `report_plan`, `report_execute` with the session's comparison as `ReportSource` and the script's
  selection as `ReportScope::FolderRows` where `options:` asks for it (W8 decides the word). `file-report <left>
  <right>` is a text report; without specs it is FC-079 (P1) and refused. `output-to:clipboard` hands
  `ReportOutcome::clipboard` to the adapter; `output-to:printer` in silent mode still opens the browser (it is not a
  dialog). Report errors follow SCR-003.
- **W9.** The folder view's commands that report go through W13's dialog; `pair_text_request` already serves `Open`.
- **W11.** The text view's selection gives `ReportScope::TextLines`.
- **W12.** `ReportKind::{Table, Hex, Picture}` with their Summary layouts (A10), `summary_counts` for each, XML version 1
  gains their `type` additively.
- **W13.** The report dialog is a model over `ReportWords`, `report_choices` and `remember_report_choices`; *View in
  Browser* = `browser`, *Print Preview* = `printer`; `Report` and `FileReport` (P1) in the catalogue as each becomes
  real.

#### 1.7.19 Documentation and version

`docs/13` (the M1 subset paragraph becomes the M2 one; new parts: the vocabulary table of §1.7.4 with the two
additions, layout × format × target, scope, Interleaved, XML dumps with the element table, versioning and the XSD's
path, links to file reports with names and the files folder, targets with clipboard and printing, remembered choices,
`SOURCE_DATE_EPOCH`); `docs/schemas/hikasa-report-1.xsd`; `docs/12` (§1 the `report` row, §3.1 the report commands'
vocabulary points to docs/13, §4 the `report` rows, `ReportArgs` on the compare subcommands and `session run`,
`--output-to`, the dry-run lines, `hikasa.report.v1` and 39 schemas, the environment `SOURCE_DATE_EPOCH`, G13
figures); `docs/04` FC-072 (one folder per aligned pair); `docs/03` §5.2 (`[dialogs."report-*"]`); `docs/02`
(`roxmltree` dev-only, `xmllint` in CI); `docs/16` (M2 progress, the reworded M3 line of D-195); `docs/17` §3 (the
mutation rule of §6.7); `docs/reference/commands.md` unchanged (`commands-doc --check` passes).

**Help** (`cargo xtask help-check` clean): new `report` (both subcommands, the vocabulary, targets, links and the files
folder, XML, printing and the clipboard's limits on Linux, `--dry-run`); updated `text-compare`, `folder-compare`
(the shared flags, `--output-to`), `session` (`session run` takes REPORT), `folder-op` (`copyto` of an aligned pair),
`json-output` (`hikasa.report.v1`, 39), `exit-codes`, `cli` (`SOURCE_DATE_EPOCH`), `index`. `hikasa help report folder`
resolves through D-67; the new topic is in `help.rs`'s list.

**CHANGELOG.** One W7 section in `## Unreleased` (Added, Fixed for D-194, Changed for M1's refusal moving before the
comparison), updated in place by each part and review, as §1.6.24. **Version.** The parts bump nothing; W7 ships as the
pre-release 0.1.93 after W7b (maintainer's standing order), and the orchestrator, not a part's implementer, bumps the
version and tags it after W7b's review (§6.7 A7.24). No product name of docs/17 §5 anywhere: grep before each commit.

#### 1.7.20 W7 notes (implementation)

*To be written by each part*, as §1.6.25: every point where the implementation interpreted or extended the text above,
numbered per part, with the mutations each property test was checked by (§6.7 rules).

**W7a.** Where the implementation interpreted or extended §1.7.2–§1.7.8 (rendering) and §1.7.17 (streaming), and why.

1. **`made` compares as the back-end does (D-194).** Rule 16's pair choice needs `made` (note 36) to hold the pair's one
   folder, so `made_key` now folds names only when the target back-end lacks `CASE_SENSITIVE`, no longer as the
   comparison does (note 42): an aligned pair whose names differ is one folder by the rule's choice, and two folders the
   comparison folds but a case-sensitive target keeps apart are two.
2. **The M1 parsers stay.** `DisplayMode::parse`, `Wrap::parse`, `TextLayout::parse`, `FolderLayout::parse` and the rest
   keep their words and their tests (`xml` and `none` are still invalid there); the W7 vocabulary lives in
   `request.rs` (`ReportRequest::parse`), which M1's callers do not reach until W7b moves them.
3. **`HtmlStyle` carries the wrap and the print block.** `HtmlStyle::{print, wrap}` are `None` by default, which is
   every byte of M1; `render_report` fills each from the request when the caller's style leaves it `None`, so a caller
   (the pair reports of W7b) can force its own. `Some(Wrap::Character)` adds no rule.
4. **`print_mono` is kept apart from `print`.** `ReportRequest::print` is made only by an orientation word or the
   printer target, as §1.7.4 says; `print-mono` alone records the print palette in `print_mono` and changes no byte of
   a file report (it is not `unused`: the browser target uses it). `apply_target` gives the printer a portrait block
   unless an orientation was given, gives the browser the print palette without a block, and puts the `html-color` or
   `html-mono` it overrides into `unused`, in slot order.
5. **New error and warning shapes.** `ReportError::Refused` (→ `unsupported`: a layout a target does not take, file
   reports without a folder on disk) and `ReportError::Request` (→ `invalid_input`: a scope without rows, a request of
   the other kind); `ReportError::is_cancelled` recognises a `CancelWriter`'s error, which classifies as `Cancelled`.
   `WarningCode::ReportOptionUnused` (`report_option_unused`) is added in `hikasa-core`; its help entry is W7b's
   (§1.7.19: `help/json-output.md` gains its warnings there).
6. **`c` outside lines.** §1.7.6's table allowed `c` only in `l`, `r`, `d`, `u`; a title, a rules name, an error, a
   status or a warning can hold the same characters (the hostile fixture has a CR and an ESC in its title), so they take
   `c` as well; the table and the XSD say so.
7. **Names that are not Unicode.** `MemoryVfs` names are `String`s, so a name that is not valid UTF-8 comes from disk
   only: Linux bytes (`cfg(target_os = "linux")`), on Windows a lone surrogate on disk (where §6.7's table says
   `MemoryVfs`, which cannot hold one), on macOS the refusal (`EILSEQ`) asserted. `raw-name` is the hex of the OS
   name's encoded bytes (`OsStr::as_encoded_bytes`: the bytes on Unix, WTF-8 on Windows). Wine cannot create a name
   with a lone surrogate (its names are UTF-8 on the Unix side): under `WINEPREFIX` the test says so on stderr and
   stops, so its Windows disk part is verified by Windows CI only, as §6.7's table has `MemoryVfs` for wine.
8. **A flat tree nests nothing.** In the folder XML of a flatten comparison (`tree/@flat`), the only folder rows are
   folders that failed to list; each is closed at once with its sides, so nothing nests, and `path` carries the
   structure. Otherwise nesting follows the comparison's tree, as P-FXML checks.
9. **`safe_component` and a trailing dot.** §1.7.8's steps leave `_CON.` for `CON.` (a leading `_` alone keeps the
   trailing dot, which `windows_name_problem` refuses), so a trailing `.` also becomes `_`, before and after the device
   prefix.
10. **`CancelWriter`.** Its first write looks at the token, so a cancel before any byte writes nothing; a write is cut at
    the next MiB boundary, so the check falls exactly there and at most `CANCEL_CHECK_BYTES` follow a cancel;
    `written()` counts what the inner writer took. A7.10's cancel test uses a sink that cancels at the n-th MiB; the
    target-and-folder half of §1.7.17's cancel row is W7b's (`report_write.rs`).
11. **How the streaming row measures.** The peak is of live heap above the live heap before the render, so the
    comparison itself (built before) is not counted; the counting `#[global_allocator]` lives in `stream_memory.rs`
    only, and a mutex runs one measurement at a time. The CI profile runs it in debug. Measured (debug, Linux): every
    text layout peaks at 67–69 KB at 50,000 and at 200,000 rows (3–36 MB written); folder side-by-side HTML and XML at
    0.46 MB for 25,250 visible rows and 1.65 MB for 101,000 (about 16 B a row: the list of visible rows), under the
    5.0 and 7.4 MB bounds.
12. **`wrap-none` frees the table.** `white-space: pre` in a `table-layout: fixed` table at 100 % would overflow its
    cells, so `wrap-none` also sets the table to `width: auto; table-layout: auto` (the page scrolls, as §1.7.4 says);
    `wrap-word` adds only `overflow-wrap: break-word` to M1's `pre-wrap`.
13. **The print block's rules.** `@page { size: …; margin: 12mm; }`, then in `@media print`: `body { margin: 0; }`,
    `print-color-adjust: exact` with its `-webkit-` twin, `thead { display: table-header-group; }`, `tr { break-inside:
    avoid; page-break-inside: avoid; }`. When the print palette differs from the screen palette, the screen palette goes
    under `@media screen` and the print one under `@media print`, before the block; the layout rules stay shared.
14. **A7.8's test file.** `tests/golden_print.rs` (not in §1.7's file list) holds A7.8: every text and folder fixture's
    side-by-side and summary HTML through `render_report`, with no wrap or print word and with `wrap-character`, equals
    the M1 renderer's bytes, which `golden_text.rs` and `golden_folder.rs` pin; no M1 HTML golden changed since 0.1.92.
    The interleaved wrap goldens are `golden_interleaved.rs`'s variants (`wrap-none-40.html`, `wrap-word-40.html`).
15. **A unit test that cannot eat memory.** `CancelWriter`'s unit test writes into `io::sink()` with a bounded loop: with
    CANCEL-1 applied, the first version (a `Vec` and an unbounded loop) grew until the machine ran out of memory.
16. **Pieces per side (D-213, W7a review).** The first `row/@piece` was the larger of both sides' piece numbers, so a
    row with the left's second piece beside the right's whole next line (limit 10, `0123456789abcdefghij` against
    `0123456789` and `abcdefghij`) said `piece="2"` of a whole right line. Each side now has its own attribute; the
    read-back of A7.5 (`xmlcheck::side_lines`) joins a side's rows by that side's attribute alone and checks the line
    number repeats, P-XML draws a limit of 1–7 in most cases and rebuilds both source strings byte for byte, and
    `xml/pieces/expected.xml` pins the bytes.

**W7a mutations** (A7.23; each applied by hand, the named test run, then reverted with `git checkout`):

| Mutation | File: change | Caught by | Seed |
|---|---|---|---|
| D194-a | `hikasa-folder/src/ops/copyto.rs`: `pair_name` returns the source side's own spelling | `ops_plan.rs` A7.1 rule-16 rows (3 tests), P-OPS aligned pairs (`props_ops_model.rs`); `cli_folder_op.rs` fails on the code before the fix | — |
| D194-b | `copyto.rs`: step 1 (the left name's `lstat`) dropped | `a7_1_rule16_one_folder_per_aligned_pair` (second run plans a `mkdir`), P-OPS aligned pairs | — |
| IL-1 | `hikasa-report/src/text/interleaved.rs`: a changed row's lines in reverse order | P-IL `p_il_interleaved_lines_are_both_documents` | `cc 59cb8f9d…` |
| IL-2 | `interleaved.rs`: the strike-out applied to the other side | P-IL (the `<del>` scan) | `cc eaf41a3d…` |
| XML-1 | `hikasa-report/src/xml.rs`: the `c` escape of a control skipped | P-XML `p_xml_text_dump_round_trips` (the parse fails) | — |
| XML-2 | `xml.rs`: `&#13;` dropped from text | P-XML (the round trip fails) | `cc d8c24016…` |
| FXML-1 | `hikasa-report/src/folder/xml.rs`: an entry closed under its grandparent | P-FXML `p_fxml_folder_dump_is_the_json_items` | — |
| LINKS-1 | `hikasa-report/src/links.rs`: `:` kept as unreserved | P-LINKS `p_links_hrefs_decode_back` | — |
| RPTREQ-1 | `hikasa-report/src/request.rs`: `words()` drops the strike-out unless both sides | P-RPTREQ `p_rptreq_words_round_trip` | `cc 23fa85ae…` |
| RPTREQ-2 | `request.rs`: of two words of one slot the later wins (no conflict) | `a7_2_conflicts` (not P-RPTREQ: its generator draws one word per slot) | — |
| CANCEL-1 | `hikasa-report/src/cancel.rs`: after the first check `next_check = u64::MAX` (the token is never looked at again) | `cancel::tests::writes_until_the_next_boundary_after_a_cancel`, `a7_10_cancel_stops_within_a_mib` (the render ends `Ok`) | — |
| STREAM-1 | `hikasa-report/src/text/mod.rs`: the interleaved text rendered into a `Vec`, then written | `a7_10_text_layouts_stream` (the bound) | — |
| STREAM-2 | `hikasa-report/src/folder/mod.rs`: the folder XML rendered into a `Vec`, then written | `a7_10_folder_layouts_stream` (the bound) | — |
| LINKS-2 | `hikasa-session/src/api/report_write.rs`: `safe_component` keeps `#` | P-LINKS `p_links_safe_names` | — |
| LINKS-3 | `report_write.rs`: the leading `.`/`-` rule dropped | P-LINKS `p_links_safe_names` | — |
| PRINT-1 | `hikasa-report/src/style.rs`: the print block never appended | `golden_print.rs`: `a7_8_print_block_goldens`, `a7_8_a_print_palette_other_than_the_screens`, `a7_8_print_mono_alone_adds_no_block_and_the_printer_target_does` | — |
| PRINT-2 | `style.rs`: a print palette other than the screen's not scoped (the screen palette only) | `a7_8_a_print_palette_other_than_the_screens` | — |

P-LINKS's generator persists no failures (`failure_persistence: None`), so its rows have no seed; the minimal input is
in the failure message. The ci profile retries a failing test twice; every row above failed all three tries.

**W7b.**

### 1.8–1.14 W8–W14

Outlines in plan §6; each section is written here before its wave starts.

---

## 6. Tests and acceptance

### 6.0 W0 acceptance (met)

- A0.1 `cargo xtask commands-doc --check` and `cargo xtask help-check` pass; the generated reference without the
  Status column equals the M1 file (458 rows; 60 `implemented`).
- A0.2 GUI tests `the_window_matches_the_core_catalogue` (both directions), `every_view_type_has_a_help_topic`,
  `help_commands_show_topics`.
- A0.3 CLI tests `cli_help.rs`: goldens of a text topic, a JSON topic and the list; an unknown topic exits 100
  with a suggestion; every M1 subcommand resolves to a topic titled `hikasa <subcommand>`; `help` works with a
  configuration that does not load.
- A0.4 Unit tests of the catalogue (validity, lookups, problems, Markdown) and of the help reader (topic set,
  resolution, inline, blocks).
- A0.5 The architecture test covers `xtask`; the workspace is green under `cargo nextest run --workspace`,
  `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check`, `cargo doc` and `cargo deny`.

### 6.1 W1 acceptance

- A1.1 **Round trip (T5, P-SET).** For every schema `T`: proptest over generated values (within ranges):
  `parse(to_toml(v)) == v`; `to_toml` of `T::default()` is exactly `schema_version = 1\n`; saving twice writes
  once (`save` returns `false` the second time).
- A1.2 **Preservation (Q11).** A fixture with unknown top-level sections, unknown keys inside known tables and
  unknown keys inside `[[app]]`/`[[preset]]` entries: after `settings set` of one key, every unknown key is
  present with an equal value (compared as `toml::Value`), and only the set key changed.
- A1.3 **Migration (SES-052).** A test-only schema with `VERSION = 3` and migrations 1→2→3 loads hand-written v1
  and v2 fixtures to the same value, sets `migrated_from`, does not write at load, writes v3 on save. A newer
  `schema_version` is `Schema` in Strict and Lenient mode; negative or non-integer is `Config`.
- A1.4 **Strict vs lenient (D-71).** Wrong type and out-of-range values: every non-`settings` command exits 100
  naming file and key; `settings list` and `settings set` work and warn `invalid_config_value`; `settings set`
  of the bad key repairs the file. M1 option fixtures (`tests` of `options.rs`) still pass.
- A1.5 **Atomic write and conflict.** Fault injection (a read-only folder, a target that is a directory): the
  target is untouched and no temporary file is left. An external edit between load and save → `Conflict`
  (exit 100); `--force` overwrites.
- A1.6 **Change detection (D-74).** `ConfigWatch` reports `Created`, `Changed` (including same length and same
  mtime with different bytes, by setting the mtime back), `Removed`, new and removed format files; nothing after
  `acknowledge`; nothing on an unchanged poll.
- A1.7 **Every key.** A test parses the "Keys" table of `docs/15` and checks that each key exists in
  `key_infos(Options)` with the same type, range or enum values and default, and vice versa; each range is
  tested at both bounds and one step outside.
- A1.8 **Schemas' extra rules.** Masks, preset names, Open With entries, `commands.toml` views, ids and
  shortcuts (every catalogue shortcut parses and round-trips its spelling), shortcut conflicts, scheme
  resolution (built-in overlay, `base`, missing scheme falls back), contrast ≥ 4.5:1 in both built-ins.
- A1.9 **Open With.** The D-76 table test (≥ 25 rows) and `InstanceMode`.
- A1.10 **CLI goldens.** `settings get|set|unset|list|restore-defaults` and each `format` write form in text and
  JSON, in a temporary `--config-dir`; read-only commands still create nothing (D-21); exit codes 0/100;
  `help settings` resolves; `cargo xtask help-check` and `commands-doc --check` pass.
- A1.11 **Format writes (D-75).** `new` (clone and empty), `edit` (file and `--set`), `delete` (user, built-in
  refused), `import` (`--replace`), `export`, `enable`/`disable`, `move` (all six forms), `restore` (one id,
  `--all`, dry run): after each, `format list --json` shows the expected registry and only the touched files
  exist.
- A1.12 The whole workspace is green (nextest, doc tests, fmt, clippy `-D warnings`, doc, deny, architecture
  test, xtask checks); the GUI builds and its tests pass, using `commands::view_sections`.

### 6.2 W2 acceptance

- A2.1 **Session files.** A TOML golden per session type (seven) and a proptest round trip per type; P1 type
  names refused; `-`/`clipboard://` specs never stored; sparse rules against the defaults resolved at save
  time (a later defaults change keeps the session's own differences).
- A2.2 **Names (D-80).** Table test: every forbidden character, reserved device names, leading `_`, trailing
  dot/space, length 0/100/101, case-insensitive collisions on save and rename, folder moves.
- A2.3 **Auto-save (D-83).** Rotation table with injected dates: group boundaries (today, yesterday, 2, 6, 7
  days, future), slot reuse, limit pruning order, 0 disables and clears, named sessions untouched.
- A2.4 **Close prompts (D-84)** and **scopes (D-82):** the two tables of §1.2.3–1.2.4 as table tests,
  including the locked parent.
- A2.5 **Workspaces.** Golden, round trip, a missing referenced session → Home tab + `session_missing`.
- A2.6 **Packages.** Export → import round trip into an empty configuration reproduces every file byte for
  byte (TOML compared as values); two exports at the same instant are identical; import plan goldens
  (collisions, deletes, summary); refusal tests for traversal, absolute and backslash paths, too many or too
  large entries, a manifest mismatch, a newer schema — and in each case nothing is written; restore plan
  goldens.
- A2.7 **CLI.** Goldens (text and JSON) for every new form; `session run` gives the same exit code and the
  same JSON as `folder compare`/`text compare` on the same inputs and rules (T11); CLI-002 resolution order
  (existing path > session > workspace > error; `.hikasa-pkg` with and without `--silent`); read-only
  commands create nothing (D-21).
- A2.8 **IPC.** Golden frames for `LoadSession`/`LoadWorkspace` and their responses; the GUI handler opens
  folder and text sessions (test through `AppCore`, no window); an `unsupported_request` answer is reported
  as D-89 says.
- A2.9 Whole workspace green (as A1.12), including `cargo deny` and the architecture test (`zip` in the
  manifest table of `hikasa-session`).

### 6.3 W3 acceptance

- A3.1 **Primitives (D-92, D-99).** One table test per primitive, run on `MemoryVfs` (case-sensitive and
  case-insensitive) and on `LocalVfs` in a tempdir (host path syntax; Windows and macOS rows run on those
  platforms, the maintainer's platform run before merge): rows for a missing item, a file, an empty folder, a
  non-empty folder, a link to a file, a link to a folder, a broken link, an existing target with and without
  `replace`, a case-only rename, a missing parent with and without `parents`, a non-folder ancestor; each row
  states the outcome or the error kind. `set_times` with and without `created` (`SET_CREATED` per platform);
  `set_attrs` `Mode` (Unix), `Dos` `R` and an unsupported bit (Windows), the wrong family. The capability table
  of §1.3.1 per back-end; the router dispatch table (local, blobs, an unsupported scheme, mixed kinds).
- A3.2 **Errors (D-93).** Every new `VfsError` variant: kind, message, `spec()`, the `io::Error` round trip;
  `from_io` of `DirectoryNotEmpty` and `CrossesDevices`; `ErrorKind::NotEmpty` serde `"not_empty"`; the §2.21
  table test still total (`vfs` → 100); `VfsOp::ALL` and `Capabilities::names` (21, canonical bit order).
- A3.3 **Copy (ARCH-031, D-95).** Table tests on `MemoryVfs` → `MemoryVfs` (two instances: another back-end),
  `MemoryVfs` → itself and `LocalVfs` → `LocalVfs`: file, empty file, folder tree, preserve on/off, created time;
  the journal shows content (`commit`) → `set_times` → `set_attrs` per file and each folder's metadata after its
  last child; a target without `SET_TIMES`/`SET_ATTRS` gives exactly one `capability_missing` warning and done
  items; a failing `set_attrs` fails the item and keeps the content; owner never changes. The local fast path and
  the streamed path give the same bytes, times and mode (a file of 64 MiB + 1 byte takes the streamed path).
- A3.4 **Faults (D-95, D-103).** Property test over generated trees (≤ 50 items) × fault position × fault op
  (`Open`, `Read`, `Write`, `Commit`, `Mkdir`, `SetTimes`, `SetAttrs`, `Symlink`, `Remove`, `Rename`, read and write
  errors after N bytes): the failing item's target is exactly as before (`dump()` equal for that path; no
  temporary sibling anywhere), every other item reaches its expected state, the summary counts match, exit code
  100. Cancellation at a random item: the running item's target untouched, later items `not_run`, `cancelled`.
- A3.5 **Overwrite (D-96).** The policy table: 4 policies × target absent / older file / newer file / equal
  `mtime` / unknown `mtime` / folder / link / special file, and source folder × target file/link/folder; the
  execution re-check (a target created between plan and execution → `changed since the plan was made`, nothing
  replaced); a source FIFO is skipped and never opened (Unix, with a timeout guard).
- A3.6 **Links (D-94).** The nine rules of §1.3.3, each a test on `MemoryVfs` and on a Unix tempdir (Windows:
  run when creating links is permitted, else skipped with a message): removing a link to a folder keeps the
  folder's content; `remove_tree` over a tree with a link to an outside folder leaves that folder intact; copying
  onto a link replaces the link and leaves the pointed-to file byte-identical; a target subfolder that is a link
  is never written through; `set_times` on a link leaves the target's times; `set_attrs` on a link is
  `unsupported`; `rename` and trash move the link; `--follow-symlinks` follows a root only; `open_write` without
  `replace_link` still writes through (M1 test kept).
- A3.7 **Move (D-97).** Same device: one `rename` item, the folder's `FileId` kept; `MemoryVfs::set_device` and two
  `MemoryVfs` instances: copy + remove per item, merge into an existing folder, a source folder kept when one child
  failed (`children_failed`); a `rename` that returns `CrossDevice` at execution is expanded and the report shows
  the expansion; a source-removal fault rolls the copy back (and keeps it when it replaced a target).
- A3.8 **Remove and trash (D-98).** `rm` table (file, link, empty folder, non-empty without/with `--recursive`, a
  root refused); `MemoryVfs::trashed()` order; a trash fault leaves the item in place and never removes it
  permanently. Linux: a CLI test runs `hikasa vfs rm --trash --yes` with `HOME` and `XDG_DATA_HOME` in a tempdir
  that also holds the file, and finds it under `$XDG_DATA_HOME/Trash/files` with a `.trashinfo` that has `Path`
  and `DeletionDate`; on other platforms the real-trash test is `#[ignore]` (manual). Blobs report
  `OperationUnsupported`.
- A3.9 **Clipboard (D-100).** With `FixedClipboard`: text, image, empty and unavailable through the Service calls
  of `quick`, `text compare`, `vfs cat`, `vfs stat` and `vfs cp clipboard:// <file>` (kinds and exit codes as
  §1.3.6); the provider is never called by a command without a `clipboard://` spec and called once for
  `quick clipboard:// clipboard://`; `read_clipboard` replaces the snapshot; text wins over an image; the image blob
  is a valid PNG (the test parses chunks, CRCs and the stored deflate blocks back to the pixels) named
  `clipboard.png`; > 1 GiB of pixels is `too_large`. CLI tests through `HIKASA_CLIPBOARD_FILE` (text, empty file,
  missing file). No test constructs `SystemClipboard` (architecture check).
- A3.10 **GUI handover (D-101).** `gui open clipboard:// x` sends `clipboard://` verbatim (fake GUI server golden
  frame); `open_view_request` accepts it and the handler path reads the injected provider (Service test with a
  fake handler); a provider error is answered `invalid_request` and exits 100; `-` is still spooled; a rules file
  `clipboard://` is still refused; `session new … --left clipboard://` still stores `""` with `spec_not_saved`.
- A3.11 **CLI (D-102).** Goldens (text and JSON) for `cp`, `mv`, `rm`, `mkdir`, `touch`: success, `--dry-run`, a
  partial failure (`ok: true`, exit 100, the failed item's error), a whole-command failure (bad spec, several
  sources into a non-folder), `rm` without `--yes` (exit 100 `invalid_input`, the tree unchanged), each
  `--overwrite` policy, `--symlinks skip`, `touch --time/--reference/--created/--create`; the `resolve_targets`
  table (existing folder, new name, trailing separator with and without `--parents`, several sources, a blob
  source, target inside source, target = source); `vfs stat` goldens with the new capability names; `version
  --json` lists 35 schemas; `hikasa help vfs cp` resolves to the `vfs` topic; `cargo xtask help-check` and
  `commands-doc --check` pass (no catalogue change). CLI results equal direct `Service::vfs_run` results (T11).
- A3.12 **Nothing is created by reading.** `vfs ls|stat|cat`, every `--dry-run`, `rm` without `--yes` and
  clipboard reads leave the tempdir, the configuration folder and the cache folder exactly as they were (tree
  snapshot before/after; D-21).
- A3.13 The whole workspace is green (nextest, doc tests, fmt, clippy `-D warnings`, doc, `cargo deny` without new
  exceptions, the architecture test with `trash` in the manifest table of `hikasa-vfs`, `arboard` in that of
  `hikasa-session` and the two confinement rules, xtask checks); the GUI builds and its tests pass (the provider
  wiring and the D-101 handler change).

### 6.4 W4 acceptance

Window tests and measurements run only under `env -u WAYLAND_DISPLAY xvfb-run -a` (D-132); every other test is
headless through the models (§1.4.16). Each part ends with its items and the full checks green.

**W4a — Shell**

- A4.1 **Windows and tabs (D-106, D-107).** Table tests through `AppCore` for every row of §1.4.2: `WindowNew`,
  tab placement (`open_new_tab_next_to_active` on/off), the target window of IPC and command-line opens (focus
  order; `open_sessions_in = "window"`), `TabDuplicate` (Home and a comparison: a second view with its own job),
  `TabMoveToNewWindow` (refused with one tab), `TabMoveToWindow`, drag moves with known and unknown positions,
  `TabCloseOthers` with and without the warning, last-tab closing both ways, a window emptied by a move closes, the
  root swap (window 1 becomes window 0, `Geometry` and `CloseViewport` effects), `Exit` warnings by window and tab
  count, `hide_tab_bar_if_single_tab`. `ViewId`s stay unique across windows; a waited view moved to another window
  still reports its D-23 outcome once (IPC test with a fake client).
- A4.2 **Layout memory (D-108).** `[layout.<view>]` and `[window]` written on window close and exit (golden
  `state.toml`), read back on the next start (fixture), precedence session `[view]` → layout → defaults;
  `state_update` keeps a concurrent external edit (edit between load and save: the retry applies both) and writes
  nothing when unchanged; `State` round trip with `layout` (A1.1 property extended); `settings get
  state.layout.text-compare` works.
- A4.3 **Commands (D-109…D-111).** Core: `effective` table tests (defaults; an override replacing, removing,
  adding shortcuts; menu/toolbar overrides; `show_all_*`; `planned` entries present but marked; the `toolbar`
  field parsed, absent = false, `commands-doc --check` unchanged in layout). GUI: catalogue parity (§1.4.16 rule
  1), menu goldens per view type with and without a fixture `commands.toml` (rule 2), the D-111 dispatch table
  (rule 3), toolbar right-click menu (mode, large buttons write `commands.toml` through `settings_save`; Hide
  Toolbar toggles `[layout] toolbar`), tab context menu entries, hint text with and without
  `show_shortcut_in_hints`, hint timing from `hint_show_ms`/`hint_hide_ms` with explicit times.
- A4.4 **Dialogs (D-112).** Prompt keys (Esc, Enter, F1 → topic), stacking, the window's commands disabled while a
  dialog is open, "Do not ask again" writes its option key, About is a dialog; the multiple-close prompts of A4.1
  answer Cancel → nothing closed.
- A4.5 **Theme and fonts (D-113).** `Palette::from_scheme` for both built-ins and a custom scheme with a `base`:
  every palette field is mapped exactly once, changing an element changes exactly its field(s); `follow_os` picks
  `light_scheme`/`dark_scheme` from the OS flag; a changed `theme.toml` re-applies once (`ApplyTheme`) and not per
  frame; font sizes reach the text panes and the tree; a family found through an injected source loads off the UI
  thread and applies once, a family not found gives one warning notice and the built-in font (D-160).
- A4.6 **Icons (D-114).** `IconId::ALL` covers the inventory of §1.4.5; every operation lies in `0..=16`; no two
  icons have equal operation lists; every presentation row on a toolbar, every session type and every notice kind
  has an icon; painted through a headless `egui::Context` at 1.0, 1.25, 1.5 and 2.0 points per pixel, the shapes
  stay inside the target rectangle and use the given colour.
- A4.7 **Status, jobs, message and log (D-115, D-116).** The job indicator aggregates the jobs of two views and
  cancels one (the other keeps running); `Stop` cancels the active view; the message panel shows, dismisses and
  honours "Do not show again"; `LogBook` table tests (each OPT-018 filter, errors always kept, capacity and
  `set_filter` shrinking, `format_line` golden, `take_unsaved` only with `save_to_file`); `log_append` creates
  and appends; `ShowLog` toggles the folder view's panel and its layout key; warnings of a folder comparison appear
  as log entries with their codes.
- A4.8 **Polling and notifications (D-117…D-119).** With `Watcher::poll_once`: each row of the §1.4.7 table (a
  changed, created and removed settings file; a changed format file → `reload_settings` and a later comparison uses
  the new registry while a running one keeps the old); the GUI's own `settings_save`/`state_update` raise no notice
  (acknowledged); an input file of a text view changed with `automatically_reload` on (reloaded, info notice) and
  off (warning notice; *Reload* reloads), removed (warning), `check_for_files_changed_on_disk = false` (nothing);
  blobs untracked; `Notices` dedupe by key, at most three visible, info hidden after 8 s, actions persist.
- A4.9 **Drag and drop (UI-008).** `drop_paths` to a path field (one and two paths), to the window (two paths
  open a comparison of the inferred kind; one and three paths give the message); drops without a path (bytes
  only) are ignored.
- A4.10 **Gates (D-132)** `text_scroll` (ARCH-GUI-003) and `two_windows` under Xvfb meet §1.4.17; the figures are
  in `docs/02`. Whole workspace green (nextest, doc tests, fmt, clippy `-D warnings`, doc, `cargo deny` without new
  exceptions, the architecture test with the new check-7 rule, `commands-doc --check`, `help-check`); no new crate
  in `Cargo.lock` (D-133).

**W4b — Sessions**

- A4.11 **Sessions of views (D-120, D-121, D-84).** Through `AppCore` with a temporary configuration: every row of
  the D-84 table reached by closing a tab, a window, `Exit`, Load Workspace, `SessionHome` and `SessionsCombo`
  (the files written or not, the prompts shown, Cancel stops each of them); IPC `CloseView` and `Shutdown`
  answer No (a dirty named session file unchanged, an unnamed view auto-saved); `SessionSave` on named, locked
  (prompt, force) and unnamed (Save As) views; Save As deletes the view's auto-saved slot; `SessionLocked`
  toggles the file; the auto-save timer with explicit times (no save before 300 s, a save after a change, none
  without a change, none for named views, pruning to `max_auto_saved_sessions`, `0` = none); FC-062 Open gives a
  child with the parent link. `session_save`, `session_autosave` and `session_save_defaults` write the same bytes
  as `session new`/`session edit`/the W2 auto-save for the same session (T11).
- A4.12 **Session Settings dialog (D-122).** Specs tab: specs, description, format, encodings (an invalid label
  refused), MRU lists pushed on OK; defaults entries have specs disabled; scope availability table (no parent,
  named parent, auto-saved parent, non-folder parent; no rule change); with a test-registered rule tab, each scope
  writes what `apply_scope` writes (files compared with the D-82 table of A2.4); OK reopens the view with new specs.
- A4.13 **Home (D-124).** Tree goldens from `session_list` fixtures (folders, favorites first, defaults, the four
  auto-saved groups, locked marks); search cases (name, folder, description, case-insensitive, ancestors shown,
  empty restores the expansion); every SES-031 command of the §1.4.11 table on a temporary configuration (files
  after each; locked refusals and force prompts; `TreeFavorite` disabled on locked); drag onto a folder moves; the
  management panel Save / Save As / Factory Defaults / Cancel; type-button drops (one path, two paths, a
  mismatch); `HomeCompare` with `clipboard://` (fixed provider) opens a view; `SessionHome` and `SessionsCombo`.
- A4.14 **Workspaces (D-125, D-126).** Save of two windows with named, unnamed, auto-saved, pending and Home tabs
  → `workspace show --json` golden; Load closes views through their close actions (a Cancel aborts with nothing
  closed), restores windows and geometry, starts only the active tabs (job counter), a pending tab starts on
  activation and is listed by `gui list`; a missing session → Home tab + notice; Manage Workspaces rename,
  delete, shortcut with a conflict shown, a workspace shortcut loads it unless a command takes the key; startup
  `load_workspace` and exit `on_exit_save_workspace_as` (fixture options); IPC `LoadWorkspace` restores windows.
- A4.15 **Command line (D-127, sessions and workspaces).** `hikasa-gui <session>` and `hikasa-gui <workspace>`
  open them and, with a GUI running, forward `LoadSession`/`LoadWorkspace` (fake-server golden frames); an
  existing path still wins (CLI-002).
- A4.16 **Gate G12** (`workspace_restore`) meets §1.4.17. Whole workspace green as A4.10; the new help topics
  pass `help-check`; `commands-doc --check` with the W4b statuses.

**W4c — Settings dialogs**

- A4.17 **Options dialog (D-128).** Every P0 key of `key_infos(FileId::Options)` is on exactly one page and no
  P1/P2 key is shown; every enum value has a caption; widgets clamp at both range bounds; search cases (a title, a
  label, a key path, an OPT id; no match → empty tree); Apply with no change writes nothing; Apply of one change
  writes only that file, sparse, unknown keys kept (A1.2 fixture), identical to `hikasa settings set` of the same
  key; an invalid value disables Apply; a concurrent CLI edit → `Conflict` → Overwrite / Reload / Cancel each
  tested; OK closes only when every file succeeded; Cancel leaves the files untouched; the window applies the new
  values (tabs, hints, log filter) after Apply.
- A4.18 **Toolbars, Shortcuts, Menus (D-129).** On a working copy: add, remove (all → `shortcuts = []`) and
  Default of a shortcut, menu and toolbar flags, display-filter mode, large buttons, show-all flags → the written
  `commands.toml` golden; live conflicts shown for a duplicate within the view and none across views; overrides of
  `planned` commands untouched; after Apply the menus and dispatch of the window follow (menu golden of A4.3 with
  the new file).
- A4.19 **Theme editor (D-130).** Round trip: change elements of a built-in and of a new scheme → Apply →
  `settings_snapshot` → `Scheme::resolve` equals the edited scheme, files sparse against their base (A1.8);
  *Reset* removes the element; New / Clone / Delete / Reset all; every edit step emits one `ApplyTheme` with the
  working scheme (preview) and Cancel emits one with the saved scheme; contrast warnings for a background below
  4.5:1; font sizes written to `theme.toml`.
- A4.20 **Wizards (D-131).** Export of chosen categories and items gives the same package entries as `hikasa
  settings export` with the same choices (manifest compared without `created`); Import steps on a package with
  collisions, a locked session (kept, then included with force) and delete-existing choices: the summary text,
  the executed results and the files equal `hikasa settings import` with the same choices; an invalid package
  stops at step 1 with nothing written; Restore plan and execution equal `settings restore-defaults --category`;
  after Finish the settings reload without a notice for the GUI's own writes.
- A4.21 **`ImportPackage` (D-127).** Golden request/response frames; the GUI shows the wizard prefilled and
  answers `Ok`; an M1-style server answering `unsupported_request` gives the D-89 message and exit 100; `hikasa
  gui open x.hikasa-pkg` exits 0 with a running GUI; `hikasa-gui x.hikasa-pkg` shows the wizard; `--silent` still
  imports headless (W2 goldens unchanged).
- A4.22 **Gates (D-132).** `options_memory` < 120 MiB PSS, then the whole set re-measured once: `text_scroll`,
  `two_windows`, `workspace_restore`, `folder_scan` (M1 acceptance (b)); all recorded in `docs/02`.
- A4.23 **Wave end.** Whole workspace green as A4.10; every W4 help topic linked and `help-check` clean;
  `docs/reference/commands.md` regenerated with every W4 status; the documentation updates of §1.4.19 done; the
  `docs/16` progress line updated.

### 6.5 W5 acceptance

Rules for every item: fixtures are **generated in the tests** by `core/hikasa-vfs/tests/common/archives.rs` (a tree as data → bytes
of each format; hostile members as raw bytes: tar headers built by hand with their checksum, because `tar::Builder` refuses
them, zip records patched after writing), never downloaded and never committed as binaries. **Every test runs on Linux,
Windows and macOS** (CI runs all three): no real links, FIFOs or devices on disk (link and device members are bytes in an
archive); no hard-coded `/tmp` or `/`-only paths (specs from `Spec::parse` on a `tempfile` folder, canonicalized because
macOS' temp folder is a link); Windows path rows use `resolve_with` and `PathStyle::Windows`, so they run everywhere; time zones
by `TimeZone::fixed` and POSIX strings, no time-zone database; readers dropped before a folder is removed (Windows); Unix-only
checks (`0700`) behind `cfg(unix)`; goldens normalize the temp prefix and separators as the existing CLI tests do. No test
reads the real clock, zone or home folder (T1). The part of each item is in brackets.

- A5.1 **Spec model (D-137) [W5a].** Table test of `Spec::parent`, `file_name`, `container`, `outermost_file`, `archive_depth`
  and `canonical` for a root, a member, a nested member and a nested root, Unix and Windows styles; `ops::check_item` refuses
  a root. Property P-ARCH-1: generated nested specs over a generated tree round-trip through `canonical`, `Spec::parse` and
  `resolve_spec`, and `Serialize` refuses a lossy base as before. `Spec::parse` never yields `ArchiveMember` (the M1 table
  of 30 rows is unchanged).
- A5.2 **Resolution (D-138) [W5a].** Every row of §1.5.4 through `resolve_with` and a table-driven fake, in both path styles
  where the row applies, on every platform; rows 1–23, 27 and 28 again through `resolve_spec` on `ArchiveVfs` over
  `MemoryVfs` and over `LocalVfs` in a tempdir (host syntax). Call counts: at most one `stat` per component; rows 4, 6 and
  28 never open the archive (`open` journal empty). Determinism: the same 1,000 specs resolved from 8 threads, in any
  order, give the same answers. A source check: `resolve.rs` names neither `std::fs` nor `std::env`.
- A5.3 **Formats (D-136) [W5a].** One generated tree in each of the ten formats (one member for the four streams), plus a
  tree with 300-byte names: `list`, `stat`, `open_read` equal the tree, on `MemoryVfs` and `LocalVfs` containers. Nested `a.zip/b.tar.gz/c.txt`
  and three levels. Concatenated gz, bz2, xz, zst. Member names of single streams (`x.txt.gz`, `noext.gz`-style cases). Masks: `.JAR`
  as zip, a disabled `tar_gz` makes `x.tar.gz` a one-member `gz`, a custom mask, an empty mask. The capability table per format
  and per nesting; the router table, with and without `with_archives`; `child` of root and folder; synthesized folders; the root's
  entry; every `Entry` column of §1.5.5. `FollowLinks::Yes` leaves a link member a link without an `EntryError`.
- A5.4 **Content reads (D-139) [W5a].** Zip members of 1 MiB and 1 MiB + 1 byte take memory and the cache (a counter). A plain tar
  member reads with no cache file and `Seek` in any order. Compressed tar: the first read decodes only up to its member and
  spools it, the second distinct read writes the whole stream once, later reads decode nothing (test counters), and a stream
  bigger than the free cache keeps spooling member by member. Every reader passes the same seek and read table. 8 threads on
  one archive give the same bytes. `tar_walk`: PAX `size` over the header, GNU long names and links, base-256 sizes, `g`
  headers, labels, an old V7 header; an extension body of 1 MiB + 1 byte is damage.
- A5.5 **Cache (D-140) [W5a].** The location for each `ConfigMode` and platform by a pure table (`resolve_config_dir`). A command
  that only lists creates no folder. Layout: numeric names, `0700` on Unix. Accounting: `used` equals the files' lengths in a
  random workload; `cache_max` evicts least recently used files, never one a reader holds; a member over the limit is
  `ArchiveLimit` and leaves nothing. Reuse: a second read decodes nothing. Invalidation: touching the container (new stamp) drops the
  index and the files. Exit: `cleanup` and `Drop` remove everything. Crash: a leftover `s7/` with an unlocked `s7.lock` goes
  at the next session's first spool; a folder whose lock is held (a second handle in the test) stays. Two `ArchiveVfs`
  on one root get different numbers. Concurrent sessions on one root, threads (`concurrent_sessions_on_one_root_keep_their_bytes`,
  `a_live_session_survives_a_crowd_of_sweeps`) and processes (`concurrent_processes_read_their_own_members`), each read
  their own bytes; a replaced cache file is decoded again; a reader keeps its bytes when the path is replaced.
- A5.6 **Names (D-141) [W5a].** Rules 1–4 and 17 as tables, for zip and tar each: `../x`, `a/../../x`, `/etc/passwd`, `\x`,
  `\\srv\x`, `C:\x`, `C:/x`, `c:x`, `..\..\x`, `a\b`, NUL, 4,097 bytes, `./a//b`, `.`, `./`, invalid UTF-8, CP437, duplicates in both
  orders, `a` with `a/b` in both orders, `A.txt` with `a.txt`; rows `<refused-N>`, the cap of 1,000, the warning's total. An
  end-to-end test extracts each hostile archive with `vfs cp -r a.zip/ out/` and asserts that the tempdir holds only what lies
  below `out/` and that the cache held numeric names only. The pure Windows name check of rule 17 on every platform, `LocalVfs::child`
  on Windows.
- A5.7 **Kinds (D-142) [W5a].** Link members: listed with their target (also `/etc/passwd`, `../..`), `open_read` →
  `OperationUnsupported`, lookups through them `NotADirectory`, a member below one refused, a copy out skips them with one
  warning and creates no link anywhere. Hard links: to an earlier member, to a later one, to a refused name, to themselves.
  Devices, FIFOs: `Other`, skipped. Sparse: listed, read refused.
- A5.8 **Limits (D-143) [W5a].** Rules 10–15 with small injected limits: a zip whose declared size is smaller and one whose
  declared size is larger than its stream; a CRC-corrupted member; a gz, a bz2, an xz and a zst of zeros over `ratio_max` once past
  `ratio_floor`, and the same with `ratio_max = 0` passing; an archive-wide ratio; overlapping zip entries stopped by the total;
  `stream_max` during an index pass; `entries_max + 1` members; `nesting_max + 1` levels, a zip that contains itself by
  name; an xz and a zst frame asking for too much memory. Each: the right error, no partial cache file, memory bounded
  (peak allocation below 64 MiB for a stream of 1 GiB of zeros with a 16 MiB `stream_max`, for gz, bz2, xz and zst, by size and by
  ratio, for a tar's index and for a read of a zip member; below the budgets of §1.5.5 for deep trees, long paths, link targets
  of 1 MiB and a central directory of long names), and an in-limit neighbour still reads. An entry that declares an absurd
  size blocks reads through the archive-wide ratio of rule 11, and the listing still works.
- A5.9 **Encrypted (D-144) [W5a].** A zip with one ZipCrypto-flagged member, one AES-flagged member (method 99) and a plain one
  (flags patched into generated bytes): names, sizes, times listed; `crc32: None`; `open_read` → `ArchiveEncrypted`; one
  warning; a folder compare gives an error row for them and a normal row for the plain one. An unknown method gives `Unsupported`.
- A5.10 **Corrupt input (D-145) [W5a].** Fuzz-style proptest, one per format of the ten, with its default case count: (a)
  random bytes under the format's mask, (b) a valid fixture with random byte flips, (c) cut at a random length. `list`, `stat`,
  `open_read` and a full read of every listed member never panic, finish, and return data or a typed error; every error
  kind is one of the four of §1.5.7. Cuts at every byte of a small `tar.gz`: members complete before the cut read correctly and
  `<damaged>` appears. `central_times` over random bytes. A decoder panic forced by a test hook becomes `ArchiveCorrupt`.
  A mask match with text (rule 8) in `vfs ls`, `quick`, a scan. Warnings arrive once through `take_warnings`.
- A5.11 **Time (D-146) [W5a].** Every row of §1.5.8. A property: a member written with a 0x5455 field reads the same instant in
  zones −12…+14, and one with only DOS time reads the wall time back in any zone. No call reads the system zone (the zone
  comes from the injected clock; a test passes two clocks).
- A5.12 **Scan (D-147, D-151) [W5a].** The mode table for: `a.zip` and `a.zip` in two trees, an archive against a folder of the
  same name, an orphan archive, a corrupt one (error row, the rest of the scan complete), a nested one. `--archives` default
  equals M1's results: the existing folder goldens do not change. `open_archives` by a path on the left, on the right, nested. Filter
  masks: a folder-exclude and a file-include against an opened archive (D-45 prefilter). Flatten. `NodeFlags::ARCHIVE`
  closed and open. The scan budget with a small injected constant: later archives stay files, one warning, also when all of them lie in one
  folder; bases and synthesized folders count. FC-025:
  zip-vs-zip with `--content crc` opens nothing (`MemoryVfs` journal), zip-vs-local reads the local side only, tar-vs-tar reuses
  the index's CRC, a corrupt member with a good central CRC is "same" for CRC and "different or error" for binary.
  `folder compare a.zip b/` and `a.zip b.tar.gz` equal the comparison of the extracted trees (states, sizes, CRCs,
  times within 2 s). `rules` TOML round trip with the two new keys, sparse (D-90).
- A5.13 **CLI, settings, gate (D-152, D-154, D-155) [W5a].** `cli/hikasa-cli/tests/cli_archive.rs`: each row of §1.5.13 marked W5a, text and
  `--json` goldens, exit codes (0, 1, 11, 100, 107), `--archives`, `--open-archive`, `switch_ignored`, unknown word → 100,
  `quick a.zip b.zip` by bytes, `vfs cat` of a link member → 100. `hikasa help archives` and `help-check`. `settings get|set
  archives.limits.*` with ranges, and a bad value refused; a broken `archives.toml` gives one warning and a working
  comparison of plain files; `reload_settings` applies new masks. D-86: a package with a bzip2 entry is still refused.
  `infer_view_kind` table. Session round trip of a Folder Compare on `a.zip/` and a Text Compare on `a.zip/x`. G6, G2 and
  the budgets of §1.5.16 measured and in `docs/02`.
- A5.14 **Batches (D-148, D-157) [W5b].** `begin`/`finish` calls in `execute` as a journal: one rebuild per operation and archive,
  nested chains, with a level nothing changes too, target roots before source roots. A second `begin` → `Unavailable`, also from
  another back-end. A primitive and an `open_write` commit outside a batch rebuild at once. A panic inside `execute`, and one in
  a change of its own, aborts (guard). Writers that race lose no update: each that reports success is in the file.
- A5.15 **Overlay [W5b].** The primitive table of A3.1, on members of a zip and a `tar.gz`, as far as it applies: rows for a missing
  item, a file, an empty and a non-empty folder, an implicit folder, a link member, existing targets with and without
  `replace`, a missing parent with and without `parents`, a rename of a subtree, across archives → `CrossDevice`, a
  single-stream archive → `OperationUnsupported`, `set_attrs` `Dos` other than `R`, `created`. The overlay is visible to
  `stat` and `list` during a batch, and gone after `Abort`.
- A5.16 **Rebuild (D-149) [W5b].** For zip and each tar format: add, replace, remove, rename, `set_times`, `set_attrs`, a folder, a link.
  Kept zip members are byte-identical compressed (same offsets of data within the raw copy, same CRC, same extra fields);
  tar members keep their raw blocks; refused, encrypted and sparse members survive; the comment survives; a shadowed
  duplicate takes each fate of D-156 (numbered and hidden records, a link to a shadow, a fresh read, the plain crates, a move
  to another archive across zip, tar and a compressed tar, and within one file). A
  renamed member keeps nothing of its old name in any ustar shape (`git archive`), a touched one keeps its name as bytes.
  Written times read back per §1.5.8; new CRCs are right. Two equal rebuilds give equal bytes. A zip with a prefix →
  `Unsupported`, unchanged; a damaged archive → `ArchiveCorrupt`, unchanged; a stamp change between plan and rebuild → items
  failed, unchanged. Property P-ARCH-2: a random script of operations on `MemoryVfs` and on an archive of the same tree ends
  in equal listings and contents, and re-reading the rebuilt archive with a fresh `ArchiveVfs` gives them too; names run
  past every field of a tar header, ustar headers included, and the archive can lie below levels nothing changes.
- A5.17 **Faults (D-149) [W5b].** Property over small trees × format × fault (`inject_read_error_after` on the container,
  `inject_write_error_after`, `Fault { op: Commit }`, cancellation at a random byte, a nested rebuild failing at each level,
  with and without a level that nothing changes): `MemoryVfs::dump()` of the tree is unchanged, so no temporary sibling
  remains on that back-end (on a disk: after a rebuild that ends, in-process; what a killed one leaves goes with the next, a
  live writer's and a user's files stay), every done item of the root is `failed`
  (`not_run` when cancelled), the summary matches, exit 100. Moves: local → archive with a failing rebuild keeps all
  sources; archive → archive with a failing target keeps the source archive unchanged; archive → local with a failing
  source rebuild keeps both copies. A later retry on the same tree succeeds.
- A5.18 **Writes through the CLI (D-150, D-152) [W5b].** `cli_archive.rs`: `vfs cp x a.zip/dir/` with and without `-p`, `cp x a.zip/`, `cp x
  a.zip` (the file, `already_exists`), `cp -r d a.tar.gz/`, `mv`, `rm --yes`, `rm --trash`, `touch`, `mkdir` inside, `--dry-run`
  plans and no change, goldens. `vfs mkdir new.zip`, each writable mask, `x.gz` a plain folder, `mkdir -p new.zip/sub` plain folders, an
  existing target; the bytes of §1.5.10; each empty archive opens and lists nothing. `cp -r` of a real folder `x.zip` stays
  a folder. `--output a.zip/out.html` rebuilds once, also three levels down (counted in `hikasa-session/tests/archives.rs`,
  `ArchiveCounters::rebuilds`: a process does not show it). `vfs mkdir x.zip/` makes a folder. Local writes unchanged: `AtomicWrite: Seek` on `LocalWriter`, and
  `DirectWriter` answers `Unsupported`.
- A5.19 **GUI (D-153) [W5a].** View-model tests through the catalogue, no window: the flag gives the icon on closed and open rows;
  `Open` on a closed row with `OnceOpened` adds the path and runs once, in `AsFiles` it opens as before; a corrupt archive
  shows the error row; a Text Compare on a member loads; `infer_view_kind` through Home's Compare; `Service::shutdown` runs at
  exit. The icon draws in the tests of D-114 (light and dark, every scale). A pointer test in `ui/tests_pointer.rs`
  double-clicks an archive row. ARCH-GUI-003 unaffected: no window gate is re-measured unless a figure moves in the folder
  gate of M1.
- A5.20 **Wave end, each part.** Whole workspace green as A4.23, including `cargo deny` (with the one exception of §1.5.15) and the
  architecture test (manifests, the confinement rule). Packaging: `packaging/build-linux.sh` and `build-windows.sh` both
  architectures once, `check-linux-binaries.sh` clean (glibc floor ≤ 2.31, baseline code), `smoke-test.sh`. Maintainer's
  run on Windows and macOS through CI before W5 merges (plan §5.1). The documentation updates of §1.5.18 done, the §1.5.19
  notes written, the `docs/16` progress line updated.

### 6.6 W6 acceptance

Rules for every item:

- **Platforms.** Every test runs on Linux, Windows and macOS unless it says otherwise; Windows and macOS are exercised
  only by CI and wine, so what differs is stated. Core logic runs on `MemoryVfs` with case sensitivity as a parameter,
  so a case rule never depends on the host: Windows and macOS temp folders are case-insensitive, and no disk test may
  assume a case. Disk tests use `tempfile`, canonicalized (macOS' temp folder is a link), with specs from `Spec::parse`;
  goldens normalize the temp prefix and separators as the existing CLI tests do.
- **Unix only (`cfg(unix)`):** modes, FIFOs, devices, sockets, `Entry::special` from disk. **Windows only
  (`cfg(windows)`):** DOS `R`, `S`, `H` set with real attributes. The rest of the attribute tables run on `MemoryVfs`
  with `Attrs::Dos`, everywhere. Real symbolic links: `cfg(unix)`, or on Windows skipped when `create_symlink` answers
  `PermissionDenied`, as A3.6; link rules are also tested on `MemoryVfs`. The four tests that fail only under wine
  (`a3_1_rename_case_only`, `a3_1_set_times_created_local`, `a3_6_rule4`, `a3_6_rule9`) are not a W6 regression; no new
  test may depend on wine's rename or file times: such a check goes behind `cfg(not(windows))` with a note, and runs on
  real Windows in CI.
- **Times.** A test sets times with `set_times` to whole seconds; never relies on a file system's granularity, nor on
  "now". Real clock, zone and home folder are never read (T1). `created` is `cfg(any(windows, target_os = "macos"))` or
  `MemoryVfs`.
- **No timers.** Tests do not sleep or race a clock. Cancellation is deterministic: a `ProgressSink` or fault hook
  cancels the token at the n-th item or byte, and the test asserts results, not time. W6 needs no wall-clock assertion.
  One that a part must add goes through `tests/common/timing.rs::meets_in_some_round(bound, measure)`, with `cancel` in
  the test's name (D-69): one round and the bound on Linux; on macOS and Windows up to six rounds, the fastest meeting
  the bound and the worst meeting G4, the rule of `core/hikasa-folder/tests/budget.rs`, because shared runners deliver
  wake-ups late for seconds at a time. Waiting on a job polls a state, bounded by count, never by a fixed sleep.
- **Faults.** `MemoryVfs` hooks of §1.3.8; `dump()` proves "untouched". New test-only hooks are allowed on `MemoryVfs`.
  Recycle bin on `MemoryVfs::trashed()`, as CI has no usable bin; a real bin stays in `tests/trash.rs`'s existing cases.
  Programs, clipboard, terminal: `RecordingLauncher`, `FixedClipboard`, and a scripted line source; no real program, no
  PTY.
- **GUI.** Headless through the models (§1.4.16); window tests only under `env -u WAYLAND_DISPLAY xvfb-run -a`.
- **Gates** run on Linux only, `#[ignore]`, by hand. Everything else is on CI. The part of each item is in brackets.

**W6a — Foundations**

- A6.1 **Batch plans (D-162) [W6a].** `core/hikasa-vfs/tests/ops_batch.rs`. A hand-built `OpKind::Batch` mixing
  `create_dir`, `copy`, `link`, `move`, `remove`, `remove_dir`, `touch` runs on `MemoryVfs`: journal order equals plan
  order; `times` per item; `Preserve` of `BatchOptions`; a failed folder cuts its subtree and no sibling root
  (`not_run`); declined `skip` blocks a parent's `remove_dir` (`children_failed`). `moves` gives two-phase behaviour
  into an archive. `execute` still equals `execute_with` with default options on every existing W3 and W5 test.
  `PlanItem` round-trips through JSON with and without the new fields, a W3 plan text reads. `hikasa.vfs_op.v1` goldens
  unchanged. A panic inside `execute_with` aborts batches as A5.14.
- A6.2 **Stamps (D-163) [W6a].** `ops_batch.rs`. Every field × source and target: changed size, time, kind, a gone item,
  a new item where none was stamped → that item fails `changed since the plan was made`, the target byte for byte
  (`dump()`), other items run. A folder stamp ignores time. `follow_source` stamps through the link. Call count: the
  journal and a `stat` counter show one `lstat` per side per item, not two. Member stamps in an archive; a member the
  plan does not touch changed by another operation since planning survives the rebuild. Windows: a local test on a file
  changed through a held handle accepts either a pass or `changed since the plan was made`, never a lost change.
- A6.3 **Durability (D-164) [W6a].** Unit tests in `local/mod.rs` count sync calls through a `#[cfg(test)]` counter: new
  target 0, replaced target 1 (file), move 2 on Unix (1 on Windows, which has no folder sync). `MemoryVfs` records the
  option. `copy_new_file` on a tempdir: a `#[cfg(test)]` stop before the rename shows the temporary sibling alone in the
  folder, already with final content, mode and time; `preserve.attrs` off takes the old path; over `FAST_COPY_MAX`
  returns `Ok(None)`. Cancel before the rename removes the temporary file. Result equals `fs::copy` + times in bytes,
  mode (`cfg(unix)`), `mtime`. Windows and macOS: `R` is set after the rename, `set_created` on macOS and Windows where
  `preserve.created`. `vfs cp` goldens unchanged.
- A6.4 **Exchange (D-165) [W6a].** Files, links (Unix), a file with a link, different sizes, across `set_device`
  devices, both sides in one archive, one side in an archive. A fault at each of the four steps of §1.6.2: rows 1–3
  leave both sides byte-identical and no hidden sibling, row 4 leaves `T2` holding the other side's content and the
  message naming it. A plain file against a member with a failing rebuild keeps both sides, no `T2` left. Two archives:
  the builder's item error (W6b). Cancel at each step. Metadata travels both ways.
- A6.5 **Fence (D-166) [W6a].** `within`: table of local paths in host syntax, a member, a nested member, an opened
  archive inside a base, a base inside an archive, a case-insensitive back-end, `a/b` against `a/bc`, the base itself
  false, `..` rows `cfg(unix)`, bounded so that a walk that does not end fails; drive-letter rows `cfg(windows)`. A link
  in a base refused at any depth, swapped in after planning on a disk, between two items of one folder for a removal; a
  link at or above a base, the item itself and a source only read go through. `execute_with`: a hand-built plan with a `remove` outside the base fails `is
  outside the base folders`, nothing removed; `writes` on and off; a `trash`, a `move` source, an `exchange`. Plan rule
  tests: A6.27.
- A6.6 **Backups (D-167, D-168) [W6a].** `core/hikasa-vfs/tests/backups.rs`. Naming: the whole table of §1.6.5, a name
  with no stem, a trailing dot, a non-UTF-8 name (`cfg(unix)`), `Folder` with `rel` and without. Execution: a replaced
  file ends at its backup with old content, times and mode; the journal order is stage, rename, commit. A second run
  replaces the backup. Faults: a stream fault leaves target and old backup; `Fault { op: Rename }` fails the item with
  target intact; `Fault { op: Commit }` puts the target back, and a second fault there names the backup in the message.
  `CrossDevice` takes the copy path, a failing `write_backup` fails the item. `link` and `create_dir` replacing. A
  folder at the backup name: planning error. Numbering of duplicates, `abc.txt` with `abc.doc`. Remove has no backup.
  Archives: one `write_backup` of the outermost file per operation, by `ArchiveCounters` and journal, also for a nested
  chain and a source-only root; none when nothing was done or on `Abort`; a failing backup aborts, the archive unchanged
  (`dump()`), items failed, exit 100; a `replaces` member has `backup: None`. Windows: rename onto a read-only backup
  fails the item, target intact.
- A6.7 **Filters (D-169) [W6a].** `core/hikasa-folder/tests/filters_attrib.rs`. Parsers: table of good and bad texts,
  TOML round trip, `false` off, sparse. `attrib` on `MemoryVfs` DOS attributes: `+r`, `-h`, `+rh-s`, on files, not on
  folders; on a Unix back-end: no effect and one `capability_missing` per side. `unix_type`: every letter on `MemoryVfs`
  with `MemMeta::special`, `Other` with none, followed links; a folder never hidden. A hidden side is teal (FC-054), and
  its folder pair still aligns. INV-FOLDER-7 by `check_tree_invariants`. Unix: a real FIFO and socket are `p`, `s`
  (`cfg(unix)`). Tar devices through `ArchiveVfs`. Windows: letters `b c p s` never match on disk; that is asserted, not
  skipped. `--attrib-filter`, `--unixtype-filter` CLI rows and an unknown letter → 100.
- A6.8 **FC-012 (D-170) [W6a].** The four rows of the table, on a fixture, states and scan status each; defaults equal
  M1 (existing goldens unchanged); verdict `incomplete`; `expanded_on_load` table, differences only, flat; a refresh of
  a `NotScanned` folder fills it.
- A6.9 **Ignored (D-171) [W6a].** `core/hikasa-folder/tests/ignored.rs`. Paths, folder inheritance, `!` exceptions,
  longest path, case by alignment. `state`, `diffs`, `content` unchanged, `category` same: verdict, exit code, display
  filter, statistics, `FolderStats::ignored`. Error and Unknown stay. `set_ignored` keeps counters equal to a recount
  (INV-FOLDER-5), and its list round-trips through a new scan to equal states; the list stays minimal. JSON flag
  `ignored`; `FLAG_NAMES` 11; TOML sparse.
- A6.10 **FF-002 (D-172) [W6a].** A rules-based folder comparison with a format off gives the result of the format that
  then wins; a child request has the overrides; an unknown id warns once and is kept; `--format-on`, `--format-off`.
- A6.11 **Refresh (D-173) [W6a].** `core/hikasa-folder/tests/refresh.rs`. Each row of the table of §1.6.7, on
  `MemoryVfs`: journal shows one `list` per side per refreshed folder, a kept subtree unread, a kept pair not re-read
  for content. Ids stable when rows are unchanged; `renumbered` right otherwise, and every read through a mapped id
  works. Counters equal a recount after each write. Side tables hold nothing for dropped ids, `content_counts` follow.
  Archive: rebuilt, re-read through the new stamp; `open_archive` makes a closed row a folder in place of a re-run.
  Failing listing: the error row. Cancel at the n-th folder: `complete: false`. `RefreshBusy`, `RefreshFlat`. Property
  P-REFRESH: random small trees, random scripts of `MemoryVfs` changes, refreshed `to_canonical` equals a fresh
  comparison, with content method CRC too. Disk variant on a tempdir: one fixed script, all platforms.
- A6.12 **Contents on demand (D-174) [W6a].** `compare_contents` on a pair and a folder, each method, a pair already
  tested counted once; budget shared (`ByteBudget` tests); a failing read is an error row; cancellation restores the old
  state, no pending rows; `compute_crcs` uses a zip's CRC with nothing decoded (journal) and reads a local file;
  `apply_child_result` with equal and changed stamps. Statistics equal a recount.
- A6.13 **`extract_to_cache` (D-186) [W6a].** Members with plain, hostile and Windows-reserved names: path inside
  `s<n>/x<k>/`, the name rule of §1.6.18, extension kept where safe. Read-only (`0400` `cfg(unix)`, `R` `cfg(windows)`).
  Same member twice, one file; a new stamp, a new file. Never evicted under pressure; a member over `cache_max` →
  `ArchiveLimit`, nothing left. `cleanup` removes it, on Windows after the read-only bit is cleared. A link, an
  encrypted, a sparse member → their errors. Cancel mid-stream leaves nothing.

**W6b — Operations**

- A6.14 **Selection (D-175, D-176) [W6b].** `core/hikasa-folder/tests/select.rs`. Word table of §1.6.9, every row,
  `lt`/`rt`, case. A fixture holding every category × file, folder, empty folder, link: matching table per side and
  `all`; `diff` with time only, size, content, override; collapsed folders; a folder with one non-matching child;
  `empty.folders`; `iu`. OR of masks; `select_all_files`; `invert` twice is the identity on shown rows; `restrict`.
  `resolve`: a folder and a child, a hidden row, filtered rows with and without `include.hidden`, `whole` on and off for
  each reason. `open_pair` table. `Finder`: substring, folded, whole word, regex, wrap, up, sides, non-matching; a bad
  regex errors. The W8 examples `left.diff.files left.orphan.files` parse.
- A6.15 **Planning (D-177) [W6b].** `core/hikasa-folder/tests/ops_plan.rs`: one test per builder rule 1–17, on
  `MemoryVfs`: pairs with different names (extension and NFC rows, case rows with a case-insensitive instance), raw
  names (`cfg(unix)` on disk; `MemoryVfs` elsewhere), missing folders once, orders, kinds, a followed source, rule 6's
  listing (journal: no `list` otherwise) with its filters, stamps, conflicts and rule 8, a target a filter hides, `whole` move as one `rename` and its expansion on `CrossDevice`, a same-named
  row of another kind, each overwrite policy, stamps, capability on a single-stream archive with one warning. The plan
  never writes (`dump()` before and after). `OpPlan` is `Clone`, serializes, reads back and executes the same.
- A6.16 **Confirmation (D-178) [W6b].** `core/hikasa-folder/tests/confirm.rs`. Conflict table: every row × file
  attribute (DOS on `MemoryVfs`, Unix mode), key on and off. `Confirmation`: order; replies per class; modes; `Cancel`
  and `No` to `Operation` → `Declined`; `finish` output per answer; a read-only yes clears `R` before replace and remove
  (`cfg(windows)` on disk, `MemoryVfs` everywhere). `delete` asks with `confirm_delete = false` through the CLI. A
  scripted line source drives the CLI's prompt path (`cli/hikasa-cli/src/confirm.rs`, unit tests: `y`, `n`, `a`, `s`,
  `c`, EOF = cancel, junk asks again). No terminal, no `--yes`: exit 100, tree unchanged. On a real terminal
  (`cli_interrupt.rs`, `script` of util-linux, Linux only): Ctrl-C at a prompt ends it at once, exit 100, nothing written;
  a file of rule 6 edited at the prompt stays. A folder that replaces a file, a read-only archive, a backup over a file of
  nobody: each asks, `no-to-all` keeps it, a text dry run shows it. On Windows stdin of a CI
  process is not a terminal, as elsewhere; the test passes a piped stdin explicitly.
- A6.17 **Recycle bin (D-179) [W6b].** Table of §1.6.12 on `MemoryVfs`: one `trash` for a `whole` folder, per item
  otherwise, the folder staying; member → `no_recycle_bin` yes and no; `trashed()`. A failing `trash` without `fallback`
  fails with the item in place; with it, the report shows `remove` items and one `trash_fallback` warning, and only what the
  plan stamped goes: a file that came since stays with its folder, a changed file stays, a `trash` with no stamped items
  fails; the same for a folder of rule 6. One `trash` asks for a read-only file below it, scanned or listed. `--recycle`
  on an archive through the CLI with `--yes`.
- A6.18 **Rename (D-180) [W6b].** `core/hikasa-folder/tests/rename_masks.rs`. Every row of the DOS table; `base_mask`
  table; a property: `dos_rename(n, "*.*") == n` and `dos_rename(n, "*") == n` for every name. Regex: groups, `${name}`,
  `$$`, `$1a`, a missing group, whole-name anchoring, `(?i)`, a backreference through `fancy-regex`
  (`EngineKind::Fancy`), a backtrack limit fails the item only. Name checks on `MemoryVfs`; `CON.txt` and a trailing dot
  fail on disk `cfg(windows)`, and `windows_name_problem` stays tested as a pure function on every platform. Collision
  table: every row; chains of 3, cycles of 2 and 3 end as asked, with no hidden name left, on `MemoryVfs` and a tempdir;
  a fault in a cycle's middle reports the hidden name.
- A6.19 **Touch, mkdir, copyto (FC-072, FC-076, FC-077) [W6b].** Rules 14–16: `OtherSide` on pairs, folders; a folder
  whose `set_times` fails is that item's failure (`Fault`). Mkdir on both sides, a missing parent, `x.zip`, `x.zip/`,
  `x.gz`. Copyto: `Base`, `Relative`, `None` per side; the common prefix in whole components (`ab/x`, `ac/y` → none); a
  new zip target created once; duplicates `skip duplicate_target`; a target inside a selected source; script example 4's
  shape (`left.diff.files left.orphan.files`, relative, a `.zip` named by date in the test).
- A6.20 **Property P-OPS (T12) [W6b].** `core/hikasa-folder/tests/props_ops_model.rs`: random trees × selection ×
  operation × answers on `MemoryVfs`, every verb that plans items (copy, move, delete with and without the bin, exchange,
  rename, touch, mkdir, copyto and moveto in each structure) with `YesToAll`, `NoToAll` and scripted per-question
  replies, with and without a name filter and unlisted folders: (a) the plan writes nothing; (b) the tree after execution
  equals, file by file (kind, bytes, time, mode), what a model of the verb computes from the tree before and the selection
  alone, never from the plan; (c) every item the plan let through runs: `not_run` only below a folder that failed, a
  failure only where the plan showed one; a share of the cases must run an item (asserted). `props_ops.rs`: conservation
  over random trees with links, filters, both cases and unlisted folders (what a verb may not touch stays, bytes are not
  lost, a declined target stays, what a filter hides stays); a refresh equals a re-comparison; cross-device moves with a
  source-removal fault lose nothing; backups hold what they replaced. Each of them fails when execution is stubbed out.
  Regressions in `proptest-regressions`.
- A6.21 **Disable editing (D-184) [W6b].** Every row of the table in §1.6.15, through `Service`, not a view:
  `ReadOnlySide`, exit 100, nothing planned; `CopyTo` allowed; a `copyto` or `moveto` whose `--target` lies at or below
  a read-only base, by the CLI and by a session, through `..`, a link at the base, a link on the way and a member of an
  archive that is the base, refused; a session's `read_only_*` and `--read-only`;
  `swap_sides`. Grep-style: `folder_op_plan` is the only caller of `plan_op` in `hikasa-session` (architecture check 7).
- A6.22 **Exclude (FC-052) [W6b].** `exclude_masks` for a file, a folder, a base file, `Type`, no extension;
  `escape_mask` property: the escaped mask matches its name and no other of a generated set, for names with `[ ? * ; !
  -` and blanks; nothing added twice; `folder op exclude --session` rewrites `[rules]`, refuses a locked session without
  `--force` (D-91), and a re-comparison hides the item; `--dry-run` prints the masks.
- A6.23 **Open With (D-186) [W6b].** `RecordingLauncher`: per item and per pair, variables through D-76, a member's
  extraction, a folder app, `accepts_files`, `wait_for_previous` order by recording, `--dry-run`, `--associated`, a
  program that cannot start (exit 100), the program's own exit code ignored. `SystemLauncher` is refused in tests by the
  architecture check.
- A6.24 **Log model (D-188) [W6b].** Golden lines of `op_log_entries` and `sync_log_entries`; OPT-018 filters;
  `LOG_DETAILS_MAX` with `… and <n> more`; failures always; time from the injected clock. The summary text equals the
  human summary (one function).
- A6.25 **`folder op` (D-185) [W6b].** `cli/hikasa-cli/tests/cli_folder_op.rs`: each verb row, text and `--json`
  goldens; `--select`, `--path`, `left:`, `--no-expand`, `--expand`, `--include-hidden`, `--display`; nothing selected →
  100; `--dry-run` writes nothing and its items equal a run's; exit codes 0, 100, 107; item failure `ok: true`;
  `--session`, flags on top; `--yes --dry-run` refused; a Ctrl-C cancels as `vfs` (`cli_interrupt.rs`, D-159: a
  `cfg(unix)` signal test, as the existing ones; Windows by the token test). `SCHEMAS` 38; `hikasa version --json`;
  `hikasa help folder op`; `help-check`. T11: `folder op copy` and the same plan through `Service` give the same tree.
  `compare`, `crc` schemas, and text and JSON goldens of `copyto`, `moveto`, `compare`, `crc`, `exclude`, `ignored`,
  `open-with`. No spelling of `--path` reaches the base. Rule flags equal `folder compare`'s by a clap reflection test. Paths in goldens normalized
  for Windows.

**W6c — Sync**

- A6.26 **Matrix (D-181) [W6c].** `core/hikasa-folder/tests/sync_matrix.rs`. **8 situations × 5 actions:** a fixture
  holds each situation; for each cell: valid cells produce the action and its effect on `MemoryVfs`, invalid ones are
  refused by `validate`, in both `orphans` readings. The five presets: the table of §1.6.14, row by row, as data and as
  an executed run. `parse`: words, kebab and snake, a base preset, repeats, unknown. Round trip through TOML.
- A6.27 **Plan rules (D-182, D-166) [W6c].** `core/hikasa-folder/tests/sync_plan.rs`. Row algorithm steps 1–6 one by
  one. `conflict` rows. Ignored rows `same`. Overrides: a path, a folder covers its subtree, an impossible one warned.
  Folders: `create_empty` on and off both ways, a wholly deleted folder, a folder keeping a filtered child; `whole`
  `trash`. Order: deletes, right, left; a file replacing a folder of its name runs. Effective rules: flatten refused,
  `within_archives` both ways, single-stream `Leave`/`Capability`. `EmptySource` refused, `--allow-empty-source`, an
  empty base with `update_*` fine; incomplete comparison refused; error rows: `sync_incomplete`, nothing below them
  touched, exit 100. Read-only sides. `SyncPlan::folder` equals a recount. One rebuild and one backup per archive in a
  sync (`ArchiveCounters`).
- A6.28 **Mirror never leaves the bases (D-166) [W6c].** Property over random trees on `MemoryVfs` with links, followed
  links (FC-013), opened archives and hostile member names: after `mirror` either way, `dump()` of everything outside
  the two bases is unchanged, whatever the filters, overrides and archive mode. Unix disk variant with a real link to a
  folder outside the base and `--follow-symlinks`: the outside folder keeps every file (`cfg(unix)`; on Windows the same
  with a folder link where `create_symlink` is allowed, else skipped).
- A6.29 **Property P-SYNC (T12) [W6c].** `core/hikasa-folder/tests/props_sync.rs`: random trees × presets × options:
  dry-run items equal executed items; after execution a re-comparison gives the plan's predicted row (mirror: no
  difference and no orphan on the rows in scope; update: no row newer on the source side); a second run plans nothing
  (idempotence); P-REFRESH holds.
- A6.30 **Sessions (D-183) [W6c].** A `folder-sync` session: `session new --type folder-sync`, `session edit --set
  rules.sync.preset=mirror_right`, an old file with an empty `[rules]`, sparse save, `effective_rules` with defaults, a
  bad cell refused, `session show`. `session run` with `--dry-run`, `--yes`, without either (100). `read_only_*` round
  trip. Package round trip (D-86). `SessionRules::Opaque` still serves its other types.
- A6.31 **`folder sync` and the boundary (D-185, D-190) [W6c].** `cli/hikasa-cli/tests/cli_folder_sync.rs`: each preset
  flag, `--custom`, `--orphans`, `--create-empty`, `--recycle`, `--within-archives`, `--override`, `--just-selection`,
  `--visible`, `--all-rows`, goldens, exit codes, `--dry-run` plans equal runs. A base missing → 107. `--sync` and
  `--fv`: `parse_open_args` table (`legacy_args.rs`). `folder sync --gui`: a fake IPC server receives `OpenView { kind:
  folder_sync }`, and an `unsupported` answer exits 100 with the message; the words are checked before any contact, as
  D-50. `--nobackups`. `hikasa help folder sync`.
- A6.32 **G7 (D-192) [W6c].** `perf_g7.rs` and `sync_plan_100k`, Linux, release, idle: thresholds of §1.6.22, recorded
  in `docs/02`. `SyncRow` size, `ops_memory.rs`. G2 and G4 unchanged. A failed gate stops the part and is reported.

**W6d — GUI hooks and release**

- A6.33 **Dialogs (D-189) [W6d].** `gui/hikasa-gui/src/tests_folder_ops.rs`, through the catalogue, no window. Catalogue
  parity (§1.4.16 rule 1) for the four commands, and the `Del` shortcut in the shortcut table and menu goldens.
  `PlanConfirmDialog`: each question, reply, "Do not ask again" writes the key, Esc = Cancel, F1 = its topic.
  `DeleteDialog`: sides, `require_side_selection_in_delete` keeps OK off, read-only sides greyed, checkboxes,
  `PermanentDelete`. A copy and a delete of a row end with the row updated, a log entry and a notice; a failed item
  gives the warning notice; Stop cancels. Commands are off for a read-only side. Refused while a dialog's job runs.
- A6.34 **Refresh in the view (D-173, D-189) [W6d].** After a copy the view's rows equal a re-run's, open folders and
  the selected row stay, through `renumbered`; `Open` on a closed archive refreshes (one `list`, no new `folder_run`),
  and the session keeps `open_archives`. Expand on a `NotScanned` folder. `expanded_on_load`. A child's result updates
  its row; a pair changed meanwhile keeps its old result. Garbage above half the arena runs a full refresh. A pointer
  test in `ui/tests_pointer.rs` deletes a row by its dialog (xvfb). ARCH-GUI-003 unaffected: the folder gate of M1 is
  re-measured.
- A6.35 **Part and wave end.** Each part: whole workspace green as A4.23 (`fmt`, `clippy -D warnings`, `nextest`, doc
  tests, `doc`, `deny`, `cargo xtask commands-doc --check`, `help-check`, architecture test), its §1.6.24 documents, its
  `CHANGELOG` entry and its §1.6.25 notes. Clippy also for the Windows and macOS targets. W6d: packaging
  (`packaging/build-linux.sh`, `build-windows.sh` both architectures, `check-linux-binaries.sh`, `smoke-test.sh`),
  pre-push scripts, and the maintainer's CI on Windows and macOS before 0.1.92 is tagged. The product name of `docs/17`
  §5 appears nowhere: grep before each commit. No part bumps the version; 0.1.92 is W6d's last step, 0.2.0 stays for M2
  complete.

### 6.7 W7 acceptance

Rules for every item: those of §6.6 (platforms, times, no timers, faults, GUI, gates), and:

- **Tests can fail.** Each property test of W7 (P-OPS's extension in A7.1, P-RPTREQ, P-IL, P-XML, P-FXML, P-LINKS,
  P-WRITE, the streaming test) is checked by mutation: at least one hand mutation of the code it guards (a named change
  in a named file: a dropped condition, a swapped side, an off-by-one, a skipped escape, a missing stamp check) makes it
  fail, and is reverted. §1.7.20 lists, per part, each mutation, the test that caught it and the seed where there is
  one; a property no mutation fails is rewritten until one does. `cargo mutants` may be run on `hikasa-report`, but its
  report does not replace the list. A check against a tool (`xmllint`, A7.7) has a negative case, so a skipped or
  stubbed check cannot pass silently where it is required.
- **Platform blind spots.** Names that are not valid UTF-8 come from Linux trees: on disk on Linux (`cfg(target_os =
  "linux")`), on `MemoryVfs` everywhere, and on macOS the disk part asserts that APFS refuses the name and stops there,
  as commit `f85fba3` did. Windows spellings of a target (`C:\…`, `\\?\C:\…`, forward slashes, spaces, `#`, `%`) run on
  disk under `cfg(windows)` and as a pure-function table everywhere. Case-insensitive file systems: Windows and macOS CI
  temp folders are, Linux's is not, and `MemoryVfs` with folding gives the same case on every host. Line endings: every
  report is `\n` on every platform (CSV `\r\n`), goldens compare bytes (`.gitattributes` `* -text`), CRLF, CR and mixed
  inputs are fixtures. Windows tests also run under wine on Linux before each push, and the heavy proptest run once
  per part, as the maintainer's pre-push rule asks; the four wine-only failures of §6.6 stay exempt and no new test may
  join them.
- **Goldens.** Every new layout × palette has goldens: text interleaved × {text, ANSI, HTML colour, HTML mono} with and
  without each strike-out, text and folder XML, folder side-by-side HTML with links × {colour, mono}, HTML wrap × layout,
  the print block × {portrait, landscape} × {colour, mono}. `HIKASA_BLESS=1` rewrites them; a review reads every
  changed golden.
- **Help and text.** Every user-visible message W7 adds (errors, warnings, dry-run lines, help) is fixed English, appears
  in a golden or a test assertion, and every warning code is in `help/json-output.md`.

**Where each runs**

| Test | Linux CI | Windows CI | macOS CI | wine |
|---|---|---|---|---|
| goldens, properties, `MemoryVfs` tests (A7.1…A7.6, A7.8…A7.10, A7.12, A7.14…A7.19) | yes | yes | yes | yes |
| `xmllint` against the XSD (A7.7) | required | skipped, one stderr line | required | skipped |
| non-UTF-8 names on disk (A7.6, A7.13) | disk | `MemoryVfs` | refusal asserted, then `MemoryVfs` | `MemoryVfs` |
| Windows target spellings on disk (A7.13) | pure table | disk | pure table | disk |
| case-insensitive file system on disk (A7.1, A7.12) | `MemoryVfs` folding; disk is case-sensitive | disk | disk | `MemoryVfs` folding |
| aligned folder pair onto a case-sensitive disk (A7.1) | disk | `MemoryVfs` | `MemoryVfs` | `MemoryVfs` |
| G13 (A7.22) | by hand, `#[ignore]` | — | — | — |

**W7a — Core**

- A7.1 **One folder per aligned pair (D-194) [W7a].** `core/hikasa-folder/tests/ops_plan.rs`, a rule-16 test per row of
  the table of §1.7.2 on `MemoryVfs` with a case-sensitive target and with a folding one, for `copyto` and `moveto`, with
  structure `base` and `relative`: items, the single `create_dir`, targets under the chosen folder's spec, a second run
  planning no `mkdir`; the same with an NFC/NFD pair under `align_normalization` on a non-normalizing target. Execution
  of each plan leaves both files in one folder (`dump()`). On a Linux disk: `hikasa folder op copyto --side both
  --structure base --case-insensitive l r --target /o/t --select all --yes` with `l/D/x.txt`, `r/d/y.txt` exits 0 and
  leaves `/o/t/D/x.txt` and `/o/t/D/y.txt` and no `/o/t/d` (`cli_folder_op.rs`). P-OPS (`props_ops_model.rs`) generates
  aligned folder pairs whose names differ in case and NFC/NFD form, on a case-sensitive target, and its model places
  both sides under one folder by the rule; P-OPS conservation (`props_ops.rs`) keeps passing. Mutation: the source's own
  spelling for items below the pair fails the unit test and P-OPS; dropping step 1 fails the second-run row.
- A7.2 **`ReportRequest` (D-196, D-209, D-210) [W7a].** `core/hikasa-report/tests/request.rs`: every word of the table
  of §1.7.4 in every spelling, each slot's default, each conflict (also flag-made words), wrong kind, unknown,
  `column-vcs` → `unsupported`, `unused` and its warning text; the layout × format and target matrices cell by cell;
  scope with each layout; M1's parser tests unchanged. P-RPTREQ: for random valid word lists, `parse(words(r)) == r` and
  `words(parse(w))` is a fixed point; any permutation of a valid list parses to the same request.
- A7.3 **Interleaved goldens (D-197) [W7a].** `golden_interleaved.rs` over every text fixture of
  `tests/fixtures/text` (35 today): `expected.interleaved.txt`, `.ansi.txt`, `.html`, `.mono.html`, and for the
  fixtures with differences `.strike-left.html`, `.strike-right.html`, `.strike-both.txt`; display all, mismatches,
  context, none; `ignore-unimportant`; `no-line-numbers`; `wrap-none` and `wrap-word` at width 40; a hidden run; CPL
  pieces; controls and bidirectional characters escaped; mark table row by row.
- A7.4 **Interleaved properties [W7a].** P-IL (`props_reports.rs`), random line pairs through `hikasa_text::compare`:
  in display all without wrap, the interleaved text with its gutter removed, keeping lines whose mark is ` ` or starts
  with `-`, equals the left document's lines in order; with ` ` or `+`, the right's; every struck span of the HTML equals
  an inline range of its side or an orphan line, and nothing else is struck (a scan of the `<del>` elements).
  Mutations: swap the order of a changed row's lines; strike the other side.
- A7.5 **Text XML (D-198) [W7a].** `golden_xml.rs`: `expected.xml` for every text fixture; a hostile fixture with NUL,
  ESC, CR inside a line, U+FFFE, U+FFFF, bidi controls, `]]>`, `&`, `<`, `"`, U+FFFD from a lossy decode, emoji
  sequences; a characters-per-line fixture (`xml/pieces`). P-XML: random pairs with generated content including those
  characters, under a random characters-per-line limit: the document is well-formed, and reading it back (`l` and `r`
  texts, `c` as their characters, `&#13;` as CR, each side's pieces joined by its `left-piece`/`right-piece`, D-213)
  gives each document exactly in display all; `summary` equals `summary_counts`, which equals the text summary's numbers; the
  `verdict` equals `hikasa.text_compare.v1`'s. Mutation: skip the `c` escape (the parse fails); drop `&#13;` (the round
  trip fails).
- A7.6 **Folder XML (D-198) [W7a].** `expected.xml` for every folder fixture (25 today), flatten included. P-FXML over
  random trees on `MemoryVfs`: the set of `(path, state)` of `entry` and `folder` equals the JSON report's items for the
  same display (D-29), nesting equals the tree (each element's `path` extends its parent's), `summary` equals
  `summary_counts`. A non-UTF-8 name (Linux disk, `MemoryVfs` elsewhere) has `name` with U+FFFD and `raw-name` whose
  bytes are the OS name; a name with controls is escaped in attributes. Mutation: put an entry under its grandparent.
- A7.7 **Schema (D-199) [W7a].** `xml_schema.rs`: roxmltree structure checks on every XML golden and 64 generated
  documents per run, everywhere; `xmllint --noout --schema docs/schemas/hikasa-report-1.xsd` on the same documents,
  required where `HIKASA_REQUIRE_XMLLINT=1` (Linux and macOS CI, set in `ci.yml`), else skipped with one stderr line.
  Negative cases: a golden without a required attribute and one with an unknown attribute are rejected by `xmllint` and
  by the roxmltree checks; a broken copy of the XSD fails. `ci.yml` installs `libxml2-utils` in the Linux test job.
- A7.8 **HTML wrap and print block (D-210) [W7a].** Every M1 HTML golden is unchanged byte for byte. New goldens: text
  side-by-side and interleaved HTML × `wrap-none`, `wrap-word`; the print block × {portrait, landscape} × {colour, mono}
  for text side-by-side, text summary and folder side-by-side; a screen palette with a different print palette.
- A7.9 **Link rendering and names (D-200) [W7a].** `core/hikasa-report/tests/links.rs`: folder side-by-side HTML with a
  scripted `PairLinks` (written, skipped, failed rows) × {colour, mono} goldens; XML `file-report`. `href` table:
  ASCII, space, `#`, `%`, `?`, `&`, `+`, non-ASCII (UTF-8 bytes), non-UTF-8 bytes, `..`, a segment of dots; every href
  decodes back to its bytes (P-LINKS). `safe_component` table (in `hikasa-session`, unit tests): controls, `/`, `\`,
  `:`, `*`, `?`, `"`, `<`, `>`, `|`, `#`, `%`, leading `.` and `-`, `CON`, `con.txt`, `NUL.x`, `COM1`, a trailing dot,
  U+FFFD, 300 bytes of `é`, empty; its output passes `windows_name_problem` and never contains a byte the href encodes
  other than non-ASCII letters (P-LINKS, over random names). Numbering: zero padding for 9, 10, 1,000 rows; two pairs
  `A.txt`, `a.txt` get distinct names whose case folds differ.
- A7.10 **Streaming (D-202, D-212) [W7a].** `stream_memory.rs` (CI, release or debug): the counting-allocator rows of
  §1.7.17 for every new layout and folder side-by-side HTML and XML. `CancelWriter`: a cancel at the n-th MiB stops
  with at most one MiB more written. Mutation: buffer the report in a `Vec` inside a renderer (the bound fails).

**W7b — Service, CLI and documents**

- A7.11 **Pair requests (D-200) [W7b].** `pair_text_request` table: exact non-UTF-8 specs (`cfg(target_os =
  "linux")` disk, `MemoryVfs`), a text rules file and patch, format overrides, ignore-unimportant, a folder row, an
  orphan, a link row → `None`. `FolderViewModel::open_request` returns what it returned before W7 on the existing GUI
  tests (`tests.rs`, unchanged) and calls the Service function (a grep in the architecture test: no `for_specs` in
  `gui/`).
- A7.12 **Linked reports on disk (D-200, D-201) [W7b].** `core/hikasa-session/tests/report_write.rs` on `MemoryVfs` and
  a tempdir: **links resolve** — every `href` of the main report, percent-decoded and joined to the main file's folder,
  names an existing file, and every pair report's back link names the main file; no other file is written. A first run
  (`New`); a re-run (`ReplaceOurs`: the old files replaced, a stale pair's file gone); a foreign folder, a folder of
  another report, a folder with one extra file, one changed file, a file at the folder's name → `_files-2`, the warning,
  and the foreign content byte for byte as before (`dump()`); 99 taken → `already_exists`. Faults (`MemoryVfs` hooks) at
  each step of the execution table and a cancel before steps 1–4 and between pairs: the main file, the folder and its
  siblings exactly as before, no hidden folder left; a fault at step 7 leaves the report and the warning. A change of
  the old folder between plan and step 4 fails "changed since the plan was made". `--dry-run`'s plan equals the executed
  plan and writes nothing (P-WRITE: random trees, random existing folders, random faults). Skipped pairs (`too_large`
  without a read, by the `MemoryVfs` journal; `binary`), a failed pair (`file_report_failed`, exit code the
  comparison's). Case-insensitive file systems (Windows and macOS CI on disk, `MemoryVfs` folding): `report_files`
  against an existing `REPORT_FILES`, two pairs differing in case, the marker's name compare. Durability: `MemoryVfs`
  records `None` for pair files and new main files, `FileOnly` for a replaced main file. A pair comparison never exceeds
  the content budget (`ByteBudget` counters).
- A7.13 **Platform spellings and names [W7b].** Linux disk: a tree with a non-UTF-8 file name and folder name, folder
  side-by-side HTML with links, XML: the row shows the lossy name, XML has `raw-name`, the pair report exists and its
  link resolves; the main file itself named with non-UTF-8 bytes keeps a working back link. Windows (`cfg(windows)`):
  `--output-to` as `C:\…\a b\r#1%.html`, `\\?\C:\…\r.html`, `C:/…/r.html`: the same files, every href relative with
  `/` and no drive or `\\?\`; the pure table of the same spellings runs everywhere. CRLF, CR and mixed inputs give
  reports with `\n` on every OS (goldens). macOS: the non-UTF-8 disk case asserts APFS's refusal, then runs on
  `MemoryVfs`.
- A7.14 **Targets and `--output` semantics (D-202) [W7b].** For every layout: a file target streams (the counting
  allocator of A7.10 through `write_output_with`); a render error, a write fault, a cancel and a refused combination
  leave an existing target byte for byte (`MemoryVfs` and disk) with no temporary file; the M1 rules hold (a link target
  replaced through it, a FIFO written directly, `cfg(unix)`); an archive member target is one rebuild; links into an
  archive, to stdout and to the clipboard → `unsupported`, nothing compared. `html-custom:<css>` reads a stylesheet
  through the VFS, over 1 MiB → `too_large`, `-` refused.
- A7.15 **Clipboard (D-203) [W7b].** `FixedClipboard` records `{text}` for each text layout and `{html, text}` for each
  HTML layout, the alternative equal to the plain-text rendering; the CLI with `HIKASA_CLIPBOARD_FILE` writes a file
  target's exact bytes (`cli_report.rs`); `NoClipboard` → exit 100; 128 MiB cap; architecture check: no `write_payload`
  in `core/` outside the clipboard module and `host.rs`. Linux helper: the hidden entry reads a payload from stdin and
  reports it placed, tested headless with a fake placer (never the real clipboard); the adapter's spawn passes the
  payload and returns after the status line while the helper runs on, and a helper that fails to start or reports a
  failure maps to `io` (a test with a helper command that exits at once); the entry is absent from `--help`, the
  reference and the help topics; a manual check on X11 and Wayland (a paste after `hikasa` exited) is recorded in
  §1.7.20.
- A7.16 **Printer and browser (D-204) [W7b].** `RecordingLauncher` receives one `open_associated` with the main file
  below `<cache>/print/`; the files equal a file target's with the implied words (A7.17); two reports in one second get
  two folders; pruning removes only folders of the pattern older than seven days by the injected clock and keeps others
  and newer ones; `NoLauncher` → exit 100, the file stays, the message names it; a layout without HTML refused before
  the comparison.
- A7.17 **Identical output (D-206, D-207) [W7b].** `cli/hikasa-cli/tests/cli_report.rs` with `SOURCE_DATE_EPOCH`, for
  every layout × palette × kind: `hikasa report …` to a file, to stdout, the shortcut `text compare`/`folder compare
  --report …`, `Service::report_execute` into a `Vec` with the same epoch, the clipboard file and the printer and
  browser files (with their implied words) are byte-identical. `SOURCE_DATE_EPOCH` table: `0`, a 2038+ value, empty,
  `-1`, `abc`, out of jiff's range (warning, the clock used); one instant for a folder report and its pair reports.
- A7.18 **`hikasa report` (D-205) [W7b].** `cli_report.rs`: both subcommands with specs and `--session` (text,
  folder-compare, folder-sync sessions), `-` and `clipboard://` sources, every flag of REPORT, `--options` and
  `--output-options` lists, conflicts, `--output` with `--output-to`, keyword-named files (`./printer`), `--lines`,
  `--select`, `--path`, `--dry-run` lines and JSON; text and `--json` goldens of `hikasa.report.v1` for each target and
  links state; `--json` with stdout → 100; exit codes equal the compare subcommand's on the same pairs (0, 1, 2, 11, 12,
  13, 100, 107 as they apply); a reflection test that the five commands flatten one `ReportArgs`; every M1 CLI golden
  of `text compare`, `folder compare` and `session run` unchanged, except where a refusal now comes before the
  comparison (D-196; each such golden named in §1.7.20); `SCHEMAS` 39 and `hikasa version --json`.
- A7.19 **Remembered choices (D-208) [W7b].** `report_choices` and `remember_report_choices` round trip through
  `state.toml`, sparse, other `[dialogs]` tables kept; a refused word dropped with the warning; a changed file between
  load and save goes through `state_update`'s retry. The CLI's output with and without remembered choices in
  `state.toml` is identical (a test writes choices that would change every slot).
- A7.20 **Help and documents [W7b].** `help/report.md` and the updated topics of §1.7.19; `cargo xtask help-check` and
  `cargo xtask commands-doc --check` clean; `hikasa help report`, `hikasa help report folder` resolve; the help list
  golden; docs/13's vocabulary table equals §1.7.4's word list (a test reads both: no word missing, none extra);
  docs/12's flag table names every REPORT flag (a clap reflection test against the doc's list).
- A7.21 **Architecture (D-211) [W7b].** Check 3 with `roxmltree` in `hikasa-report`'s dev list and `Cargo.lock`'s
  package count unchanged; check 2 and check 7 rules of §1.7.16, each with a test that a planted violation is found
  (the existing `check_*` self-tests pattern).
- A7.22 **G13 (D-212) [W7b].** The rows of §1.7.17, Linux, release, idle; figures in docs/12 beside G1 and G2. G1, G2,
  G4 re-measured unchanged. A failed gate stops the part and is reported with its figures.
- A7.23 **Mutation record [W7a, W7b].** §1.7.20 holds, per part, the list the first rule asks for: each property and
  integration test of A7.1, A7.2, A7.4…A7.7, A7.10, A7.12, A7.17 with at least one mutation that failed it. A reviewer
  re-runs two of them.
- A7.24 **Part and wave end.** Each part: whole workspace green as A6.35 (`fmt`, `clippy -D warnings` also for the
  Windows and macOS targets, `nextest`, doc tests, `doc`, `deny`, `commands-doc --check`, `help-check`, architecture
  test), the Windows tests under wine and the heavy proptests before a push, its §1.7.19 documents, its CHANGELOG
  entry and its §1.7.20 notes. W7b: the maintainer's CI on Windows and macOS; packaging scripts unchanged and
  `smoke-test.sh` passing. No version bump by a part; the orchestrator bumps to 0.1.93 and publishes the pre-release
  after W7b's review (§1.7.19). The product name of
  docs/17 §5 appears nowhere: grep before each commit.
