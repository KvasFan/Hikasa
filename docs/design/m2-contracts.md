# Hikasa M2 — Binding Contracts

> **Status: binding** for milestone M2 (version 0.2, `docs/16-roadmap.md`), branch `feat/m2`. It **extends**
> `docs/design/m1-contracts.md`: every tenet (T1–T11), decision (D-01…D-58), contract and test rule of M1 stays
> in force unless a decision below revises it with a new id. The plan, the scope and the maintainer's answers are
> in `docs/design/m2-plan.md` (§4.1); where the plan and this document disagree, this document wins.
>
> The contract is written **wave by wave** (plan §2): each wave's section is completed before the wave starts, so
> its implementers code against settled types. Sections of later waves are outlines until then (plan §6).

## Table of contents

- §0 Ground rules: scope, new tenets, decision register D-59…, dependencies, documentation updates
- §1 Wave contracts — §1.0 W0 Infrastructure (complete) · §1.1 W1 Settings store · §1.2 W2 Sessions, workspaces, packages ·
  §1.3 W3 VFS mutations, clipboard, generic copy · §1.4–§1.14 W4–W14 (outline in plan §6, written before each
  wave)
- §6 Tests and acceptance per wave — §6.0 W0 · §6.1 W1 · §6.2 W2 · §6.3 W3

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
| D-80 | **Session files** (SES-001…006, ARCH-040). A session is one TOML file loaded and saved by the D-70 engine: named sessions `sessions/<folder path>/<name>.toml`, defaults `sessions/_defaults/<type>.toml`, auto-saved `sessions/_autosaved/<id>.toml`. Names and folder names: 1–100 characters, none of `/ \ : * ? " < > |` or control characters, no leading or trailing space or dot, not starting with `_`, not a Windows device name (`CON`, `NUL`, `COM1`, …); unique per folder **case-insensitively**. A session reference is the folder path and name joined by `/` (`work/api-v2`). | Portable names on every platform; `_` keeps the reserved folders apart. | 03 §1, §5.3 |
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
| D-91 | **Locked sessions and bulk commands** (SES-004). `settings import` does not replace a locked named session, nor delete one with `--delete-existing sessions`; `settings restore-defaults --delete-sessions` does not delete one. Each is planned as `skip_locked` (never executed, no result, not counted in the summary) with warning `session_locked`; `--force` on `settings import` / `restore-defaults` includes them. `session delete` and `session rename` of a locked session are `invalid_input` unless `--force`; auto-save never writes a locked file. The defaults files are never locked. | Maintainer, 2026-10-29: a lock protects a session from every command that would change it by accident. | 03 §1, §6, 12 |
| D-92 | **Mutating VFS = single-item primitives + one generic layer** (VFS-001, ARCH-031). The `Vfs` trait gains `create_dir`, `remove` (file, link or empty folder), `rename` (one back-end), `set_times`, `set_attrs`, `create_symlink`, `move_to_trash` and the optional `copy_file_fast`, each with a default body returning `OperationUnsupported` and each behind a capability bit (seven new bits, `SET_TIMES` now used). Recursion, cross-back-end copy, move, plans and per-item results live once in `hikasa_vfs::ops` over the primitives. `MemoryVfs` implements every primitive with fault hooks (§1.3.8). | One owner of the algorithms (T11); `MemoryVfs` faults test them for every back-end; W5 archives only implement primitives. | 01 §3.2, 11 §1 |
| D-93 | **Errors of mutations.** New `ErrorKind::NotEmpty` (`not_empty`); `VfsError::{NotEmpty, CrossDevice, OperationUnsupported, InvalidTarget, Unavailable}` with the kinds of §1.3.2 (`CrossDevice`, `OperationUnsupported`, `Unavailable` → `unsupported`; `InvalidTarget` → `invalid_input`); seven new `VfsOp` names. Exit codes unchanged: any `vfs` error or failed item is 100. | "Folder not empty" is a common, user-visible outcome that deserves its own kind; the rest maps onto existing kinds. | 11, 12 §2 |
| D-94 | **Writes never follow the final link** (plan §5.1 "never follow a link being deleted"). `remove`, `rename`, `move_to_trash`, `set_times` act on a link itself; `set_attrs` on a link is unsupported; a copy or move onto a link replaces the link (`WriteOptions::replace_link`), never the file it points to; trees are never descended or written through links; links are copied **as links** (`LinkPolicy::Copy`, or `Skip`), root arguments are followed only with `--follow-symlinks`. `open_write` keeps the M1 follow-the-link save semantics by default. | A copied tree stays faithful, never pulls data from outside the tree and cannot loop; deleting through a link is the classic data-loss bug. | 11 §1 |
| D-95 | **Generic copy** (ARCH-031). Per file: `open_read` → `open_write` to a temporary sibling → `commit` (atomic rename); any read/write/cancel fault drops the writer, so the target is untouched. Local → local uses `std::fs::copy` into the temporary sibling for files ≤ 64 MiB (larger files stream, to keep cancellation ≤ 100 ms). Metadata after content, in the order times, then attributes; a folder's metadata after its subtree; owner never. A missing capability skips the step with one `capability_missing` warning per operation; a failing supported step fails the item and keeps the content. | ARCH-031 text; a read-only result must not block the time change; the OS copy is not cancellable. | 01 §3.2, 11 §1 |
| D-96 | **Overwrite policy** `fail` (default) / `skip` / `always` / `newer` (strictly newer `mtime`; unknown → not newer). A link at the target counts as a file; a folder is never deleted to make room for a file; folders merge. Special files (FIFO, socket, device) are skipped, never opened. At execution an item planned without a target that finds one fails ("changed since the plan was made"). | Safe default for a headless tool; execution never does more than the plan showed (T12). | 11, 12 |
| D-97 | **Move.** Same back-end, rename capability, target absent → one `rename` (atomic, O(1)); known different devices, another back-end or an existing target folder → copy + remove per item (folders merged, a source folder removed only when empty). A `rename` failing with `CrossDevice` at execution is expanded in place. When removing the source fails, the fresh copy is removed again unless it replaced an existing target. | Rename where possible; no duplicate and no loss otherwise; matches the W6 acceptance "copy + delete with rollback". | 11 |
| D-98 | **Remove and trash** (FC-073 primitive). `rm` removes files, links and empty folders; non-empty folders need `--recursive` (also with `--trash`); recursion is post-order and keeps a folder whose child failed (`children_failed`). Trash is `LocalVfs` only, through `trash` 5 (`chrono` for the freedesktop `DeletionDate`), one call per root item, links trashed as links, and **never falls back to a permanent delete**. A file-system root is never removed or moved. `trash` reads the home/data folders and the clock inside the call: accepted as an OS side effect (not a result input, T1), confined to `local/trash.rs`; tests isolate it with `XDG_DATA_HOME`/`HOME` in a subprocess. | Recoverable deletes must be recoverable or fail; T1's purpose (deterministic results) is kept. | 11 §1, 02 |
| D-99 | **Times and attributes** (FC-076 primitive). `set_times` sets the modification time and, where `SET_CREATED` (Windows, macOS), the creation time; the access time is never changed. `set_attrs` sets Unix permission bits (`std`) or DOS bits; on Windows only `R` is settable (`H`, `S`, `A` need Win32 calls, i.e. `unsafe`, which is forbidden); owner and group are never changed. `touch` defaults to the injected clock's now. | std-only, no `unsafe`; FC-075 (attributes dialog) is P1. | 11 §1, 04 §7 |
| D-100 | **Clipboard back-end** (TC-002, TC-065 data). A pure `ClipboardProvider` trait in `hikasa_session::clipboard`; `host::SystemClipboard` (adapter-only) reads through `arboard` 3 (text, else image, else empty). The service reads it lazily on the first `clipboard://` spec and once per service; `Service::read_clipboard` re-reads (GUI). Text → UTF-8 blob without a name; image → an in-house stored-deflate PNG named `clipboard.png`. No clipboard (headless, CI) → `unsupported`, empty → `not_found`, both exit 100. Tests use `FixedClipboard` or `HIKASA_CLIPBOARD_FILE`, never the real clipboard. | Q9 (`arboard` approved for the CLI too); the VFS stays free of OS clipboard calls; no image encoder dependency before W12. | 10, 11 §1, 12 §4 |
| D-101 | **Revises D-49.** `gui open clipboard://` and `--gui` with `clipboard://` are allowed: the CLI sends `clipboard://` verbatim in `OpenView.specs`; the GUI reads **its own** clipboard (`Service::read_clipboard`) when it opens the view. `-` keeps the M1 spool; rules files may still not be `-` or `clipboard://`; sessions still never store either (D-80). | `arboard` approved (Q9); the GUI runs on the user's desktop while the CLI may be headless. | 12 §1.1, 01 §5.1 |
| D-102 | **`hikasa vfs cp\|mv\|rm\|mkdir\|touch`** (§1.3.7). Target naming as `cp`/`mv` (into an existing folder, else the new name; several sources need a folder), owned by core (`ops::resolve_targets`). `rm` needs `--yes` unless `--dry-run` (as D-77); the other four need no confirmation because they replace nothing without an explicit `--overwrite always\|newer`. One schema `hikasa.vfs_op.v1`; item failures give `ok: true`, exit 100 (as W2 import); human output one line per item plus an ARCH-101 summary. | CLI-first (T10) before W6/W9 use the same primitives; destructive commands never run by accident. | 12 §1, §4 |
| D-103 | **Plans of VFS operations** (T12, minimal). Every write command builds the complete serializable `ops::Plan` (whole source trees listed, items in execution order) before the first write; `--dry-run` prints that plan; execution re-checks each target and never exceeds the plan; results are per item (`planned`, `done`, `skipped`, `failed`, `not_run`). W6's `OpPlan` wraps these items (sides, conflicts, backups, confirmation). | The plan is the unit W6 extends; a finite plan also makes "copy a folder into itself" harmless. | 12 §4 |

### 0.4 Dependencies

W0 and W1 add no third-party crate (the change stamp uses `hikasa_vfs::crc32_reader`). Member manifests (architecture check 3): + `xtask` = `[hikasa-session]`. The
approved crates of D-63 enter the table of the wave that first uses them. **W2** adds `zip` (D-63) to `hikasa-session` for settings packages, with the workspace-level features `deflate` only (pure-Rust back-end; W5 adds the archive features); `docs/02` gains its line.
**W3** adds `trash` 5 to `hikasa-vfs` (`default-features = false`, features `chrono`, `coinit_apartmentthreaded`;
called only from `local/trash.rs`) and `arboard` 3 to `hikasa-session` (`default-features = false`, features
`image-data`, `wayland-data-control`, the set `Cargo.lock` already compiles through `egui-winit`; named only in
`host.rs`). Member manifests: `hikasa-vfs` += `trash`, `hikasa-session` += `arboard`; check 2 gains the two
confinement rules of §1.3.9; `docs/02` gains both lines; `cargo deny` must pass without new exceptions (D-98,
D-100).

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
| `settings_unset(&SettingsKey)` | removes the key (back to default); same DTO |
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
the absolute path (the crate canonicalizes the parent only, so a link is trashed as a link; D-98).
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
existing folder"`, `"needs a file name"`, `"is a root folder"`, `"changed since the plan was made"`.
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
   with `std::fs::copy` into a temporary sibling created exactly as `open_write` creates one, then committed with
   the same rename protocol; larger files return `Ok(None)` (streamed), because `std::fs::copy` cannot be
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
pub fn plan_copy(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &CopyOptions, cancel: &CancelToken)
    -> Result<Plan, VfsError>;                       // Err only for Cancelled
pub fn plan_move(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &MoveOptions, cancel: &CancelToken)
    -> Result<Plan, VfsError>;
pub fn plan_remove(vfs: &dyn Vfs, specs: &[Spec], opts: &RemoveOptions, cancel: &CancelToken) -> Result<Plan, VfsError>;
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
`children_failed`). Cancellation is checked between items and inside copies: the running item fails with
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
error; when it replaced one, the copy stays and the item fails.

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
  `core/hikasa-session/src/host.rs` under `core/`. `SystemClipboard` and `clipboard_provider` join `capture_env`
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

*W3 note:* where the implementation had to interpret or extend the text above. (Filled in during the wave.)

### 1.4–1.14 W4–W14

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
