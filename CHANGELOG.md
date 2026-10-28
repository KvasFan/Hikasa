# Changelog

## Unreleased (M2, branch `feat/m2`)

M2 is planned in `docs/design/m2-plan.md` (the maintainer's answers in §4.1) and specified wave by wave in
`docs/design/m2-contracts.md`. Entries per wave, newest first.

### Wave 1 — settings store
- **One engine for every settings file** (D-70, T13): `options.toml`, `state.toml`, `commands.toml`,
  `theme.toml`, `themes/<name>.toml`, `filters.toml`, `openwith.toml` and `archives.toml` are typed
  (`hikasa_session::settings`), carry `schema_version` with a migration chain (older files are written back on
  the next save, newer ones refused), keep unknown keys, are written sparsely (only values that differ from the
  defaults) and atomically, and refuse to overwrite a file changed on disk since it was read (`--force`).
- **Every option of `docs/15`** (OPT-001…OPT-022, D-71): `options.toml` is read completely; a wrong type or a
  value out of range is an error naming the file and the key for every command except `hikasa settings`, which
  warns `invalid_config_value` (a new warning code) and uses the default so that `settings set` repairs the file.
  Entries of `[[app]]` and `[[preset]]` are never dropped or reordered by that repair.
- **`hikasa settings get|set|unset|list|restore-defaults`** (D-73, D-77) with keys `<file>.<path>`, suggestions
  for unknown keys, shortcut-conflict warnings, a `--dry-run` plan and `--yes` for deleting files; schemas
  `hikasa.settings_get.v1`, `hikasa.settings_set.v1`, `hikasa.settings_list.v1`, `hikasa.settings_restore.v1`.
- **`hikasa format new|edit|delete|import|export|enable|disable|move|restore`** (D-75): they write only the files
  they change; a built-in can be disabled or restored, not deleted; the id `order` is reserved; schemas `hikasa.format_write.v1`,
  `hikasa.format_export.v1`, `hikasa.format_restore.v1`.
- **Change detection** (D-74): `ConfigWatch` polls length, time and CRC-32 of every settings and format file.
- **Themes** (D-78): built-in `light` and `dark` schemes (the M1 palettes) with every element of UI-021…023,
  editable by override files, custom schemes with a `base`; contrast of default text ≥ 4.5:1 is tested.
- **Shortcuts, Open With, archive types, filter presets** in core: the shortcut spelling of the command
  reference (every catalogue shortcut parses), per-view customization and conflicts (UI-003); pure Open With
  command expansion with quoting (D-76); archive type masks (OPT-010); name filter presets (OPT-020).
- Help topic `hikasa settings`; `hikasa format` extended. The window takes the sections of its view types from
  `hikasa_session::commands::view_sections`.

### Wave 0 — infrastructure
- **Command catalogue in core** (D-65): every command of the reference (458 in 14 sections) is data in
  `core/hikasa-session/catalogue/commands.toml`, read by `hikasa_session::commands`, with a status
  `implemented` (the 60 the window executes) or `planned`. `docs/reference/commands.md` is generated from it by
  `cargo xtask commands-doc` and gains a Status column; without it the file is the M1 one (D-66).
- **Help topics** (D-67): 17 Markdown topics in `help/` (contents, command-line basics, exit codes, JSON output,
  one per subcommand, the main window and the three views). `hikasa help [topic…]`, `hikasa help --list`, schema
  `hikasa.help.v1` (appended to `hikasa version --json`); an unknown topic exits 100 with suggestions.
- **The window**: Help ▸ Contents and Help ▸ Context Sensitive Help (F1) open the topics in a resizable panel on
  the right; links open other topics; Esc closes it. Tests check that the window executes exactly the
  `implemented` catalogue entries, with the catalogue's shortcuts.
- **Tooling** (D-68, D-69): the `xtask` workspace member (`cargo xtask commands-doc [--check]`, `cargo xtask
  help-check`); CI runs `cargo nextest run` (latency tests one at a time, retried on CI) plus doc tests, and a
  `docs` job runs the two xtask checks; `bench.yml` runs the benches and the G1/G2 gates by hand.
- Decisions D-59…D-69 record the maintainer's answers (xz through the pure-Rust `lzma-rust2`; no `uzers`).

## 0.1.0 — 2026-10-26 (M1)

M1: the core engines, the `hikasa` command line and the GUI prototype (`docs/16-roadmap.md`). Entries per wave,
newest first.

### Windows and macOS port fixes
- Opening a folder as a file is `is_a_directory` on Windows too (`LocalVfs::open_read`, `--log`; Windows reports
  it as "access denied"), and the mapped binary compare and CRC fall back for a folder instead of failing.
- macOS (and every Unix without `/proc`): the effective uid is found also when several threads capture the host
  facts at once; a shared probe file name made all but one report the uid as unknown, and the IPC socket folder
  check then refused the endpoint (`InsecureEndpoint`).
- A path flag given twice (`--config-dir cfg` and `--config-dir <cwd>/cfg`) is the same path also when it does not
  exist yet behind a symbolic link (macOS `/var` → `/private/var`).
- IPC on Windows: a client's connect to a busy pipe waits at most the call's timeout (`IpcError::Timeout`, exit
  100) instead of forever, and a server shut down right after it started no longer leaves its accept thread
  blocked (the self-connect stays open until the accept thread ends; contract §1.8.7 item 13).
- The repository checks files out byte for byte (`.gitattributes`: `* -text`), so goldens with CRLF or LF line
  endings stay exact on Windows; CI runs `cargo test --workspace --no-fail-fast`. CLI goldens mask the
  platform's file metadata, case sensitivity and capability lists outside Linux (contract §6.1).

### M1 final review fixes
- `hikasa-gui` binds its IPC endpoint and opens the window before it writes the first-run format files
  (`FormatStore::materialize`, D-21); the files are written on a background thread after the first frame, so
  disk flushes on a slow or busy disk can no longer delay the window or make `gui open --wait` miss its launch
  budget (unit tests with a blocking fake task and for the startup path; `tests/cli_gui.rs` checks that the
  files appear after the launch).
- Window tests run on a virtual X display only: `env -u WAYLAND_DISPLAY xvfb-run -a …` (`docs/17` §3.5);
  `tests/cli_gui.rs` and `tests/perf_gate.rs` remove `WAYLAND_DISPLAY` from the processes they start whenever
  `DISPLAY` is set. `docs/02` §2: the long non-ASCII scroll run was on the real display, not under Xvfb.
- CI no longer installs GTK/WebKit packages (eframe needs none to build); the `--gui` help text no longer shows
  an internal decision id; README, `docs/00` and `docs/16` describe the implemented state and the open release
  items; crate descriptions say what each crate contains in M1; contract §0.4 rule 3 (`anyhow` only in the
  GUI binary) and §6.4, `docs/02` and `docs/17` (CI bench job, `cargo xtask commands-doc` and nextest are M2
  items); `docs/12` on `quick --size` and the folder report defaults.

### Wave 6b — the GUI on egui
- `hikasa-gui` moved from Dioxus 0.7 to **egui 0.37 with eframe** (D-56 as amended, ARCH-GUI-001): the Dioxus
  prototype met the first screen and the frame rate but not the ARCH-GUI-003 memory limit (598 MiB PSS, 456 MiB
  GUI RSS), so the maintainer moved the whole GUI. The view models, the command catalogue, the IPC handler and
  single instance, the D-23 close codes, Home's rules and FC-062 are unchanged; the page, its script and styles
  (`assets/`) and the page ↔ Rust request channel are gone.
- One native window, repainted on demand (the mailbox wakes it with `request_repaint`; no polling thread): menu
  bar, tabs, toolbar (with the display-filter and folder-mode lists, the toggles and Ignore unimportant) and
  status bar generated from the catalogue; light and dark colours follow the system. Keyboard shortcuts are
  taken at the window root, the focus-bound keys (Enter, Left, Right, arrows, pages) by the folder tree while it
  has the focus; Ctrl with `+`/`=`, `-` and `0` stand for the keypad keys of the font-size commands.
- *Text Compare*: the panes are custom-painted for the visible rows only (line numbers, important, unimportant
  and orphan backgrounds, hatched absent sides, inline highlights, syntax colours, gap bars that expand on a
  click, the cursor row), with a thumbnail (click or drag to jump), a scroll bar, wheel and keyboard scrolling;
  the scroll position is a row index, so million-row views scroll exactly. *Folder Compare*: the tree is painted
  the same way (colour spots, names, sizes, times, centre marks with tooltips, disclosure triangles, selection,
  hover), updates while scanning, and opens a pair with Enter or a double-click.
- ARCH-GUI-003 re-measured 2026-10-26 (desktop, Radeon RX 7900 XTX on Vulkan, 120 Hz display; `docs/02` §2): first
  screen 0.51 s after the process start (coloured 0.70 s); 119.6–119.9 fps in all three scroll phases (UI work
  per frame ≤ 5 ms, p95 ≤ 3.4 ms); memory 275 MiB PSS / 281 MiB RSS after scrolling (peak ≈ 365 MiB) against
  381.5 MiB — **all three items met** on this machine (not yet on a 2020-class laptop). An empty window takes
  76 MiB PSS (Dioxus: 168). Folder scan with every folder open: 100,020 files per side in 0.21 s with UI work
  ≤ 3.3 ms per frame; 400,080 files with a warm cache ≤ 11.8 ms per frame.
- Tests: 39 unit tests (the mailbox's notifier, egui key → catalogue shortcut, UTF-16 columns → layout
  sections replace the Dioxus request-query and ticker tests); the window test runs under Xvfb (with
  `env -u WAYLAND_DISPLAY`, see the final review fixes); the benchmark
  summary now reports per phase the frame work, the frame intervals and eframe's CPU time per frame.
- Review fixes (wave 6b verification):
  - The benchmark reads PSS and RSS from one `smaps_rollup` snapshot and only the peak from `VmHWM` (the RSS came
    from the approximate `VmRSS`, and the unit test that compared it with the PSS failed now and then); memory
    re-measured on the same desktop: 275 MiB PSS / 281 MiB RSS, peak ≈ 365 MiB (`docs/02` §2).
  - Text panes: the UTF-16 column table of a row is built in one pass, so long non-ASCII lines no longer cost
    quadratic time per frame (a 4,096-column regression test).
  - `hikasa-text` gains `TextComparison::next_section_from` and `TextDisplayFilter::toggles`/`toggled` (additive,
    contract §1.5.7); the GUI no longer re-derives the importance rule of section navigation or the filter →
    toggles table. Toggling a category in *Show Context* now keeps the context rows (only switching *Same* off
    removes them); *Same* shows as checked there.
  - Names with control or bidirectional characters are escaped in tab captions (`escape_name`, as `gui list`
    prints them), pane headers, the folder base strip and error strips (`escape_controls`). The text panes show
    C1 controls, bidirectional formatting characters, zero-width characters, LS, PS and BOM as `\u{…}` escapes.
  - A job wakes the window after its body returned and the window repaints at once until the result is
    published, so a finished comparison no longer waits for the 100 ms poll.
  - Closed views release their data (a unit test); the memory a long-running window keeps after many
    open/close cycles is glibc's retention of freed heap, bounded and documented in `docs/02` §2.
  - README, `docs/02`, `docs/05` and the contract no longer name Dioxus as the toolkit or an approved crate.

### Wave 6 — the GUI prototype (Dioxus, replaced in wave 6b)
- `hikasa-gui`: the desktop window on Dioxus 0.7 (D-56, ARCH-GUI-001) over the session service (contract §1.10,
  §3.7); it contains no comparison logic (ARCH-L3, architecture check 7). One window with tabs:
  - *Home*: two path fields and Compare, resolved like the command line (`resolve_open`, CLI-002) on a job
    thread; the paths are passed verbatim (spaces are legal in names), and `-` and `clipboard://` are refused
    with an inline message (the window never reads standard input).
  - *Folder Compare*: a virtualised tree (only the rows on screen exist in the page) with per-side colour spots
    and names (FC-031, D-55), size and time columns, centre state marks, open/close folders, Expand All and
    Collapse All, the 13 display filter presets, the six toggles and the three folder modes (FC-055, FC-056),
    Ignore Unimportant Differences and Suppress Filters (FC-057), next/previous difference (FC-061), Stop, Full
    Refresh, and Open (Enter or double-click) of a file pair in a Text Compare tab with the exact paths of both
    sides (FC-062). The tree is rebuilt from the live model while the scan runs (at most every 66 ms).
  - *Text Compare*: side-by-side panes drawn on a canvas (ARCH-GUI-002): the page asks for the slice of rows it
    is about to draw and the Rust side lays out only that slice (line numbers, important, unimportant and orphan
    colours, inline highlights, syntax colours per grammar element, gap bars that expand on click); a
    thumbnail (TC-032), next/previous difference section (TC-050), display filters with Show Context and the
    toggles (TC-040), Ignore Unimportant Differences, font size, Reload. The documents appear unaligned as soon
    as they are loaded (`CompareObserver::loaded`), the aligned comparison replaces them.
- Command catalogue (ARCH-070, UI-003): the ids of `docs/reference/commands.md` (plus `HomeCompare`), captions,
  hints, default shortcuts, menu and toolbar placement, radio groups and availability predicates; the menu bar,
  the toolbar and the keyboard are generated from it.
- IPC server and single instance (ARCH-090…092, §4.4): `OpenView` (text and folder views, `--wait`),
  `ListViews`, `CloseView`, `Ping`, `Shutdown`; a closed view reports its D-23 code through
  `ViewOutcome::close`; quitting (IPC, Exit, the window's close button) closes every view with reason
  `shutdown` and its real code, then shuts the server down; a second `hikasa-gui a b` forwards its request to
  the running instance and exits 0 (`--solo` runs alone).
- Tests: 32 headless unit tests drive the view models through the catalogue (the enabled state of every
  command, navigation, filters, slices, FC-062, Home, the D-23 outcomes) and the IPC handler with a real
  in-process server and client; `tests/cli_gui.rs` (ignored: it opens a window) runs the real `hikasa` against
  the real `hikasa-gui` (`gui open --wait` codes of a text and a folder view, `gui ping/list/close/quit`,
  second-instance forwarding); `tests/perf_gate.rs` (ignored) measures ARCH-GUI-003 and the folder scan through
  the scripted benchmark mode (`HIKASA_GUI_BENCH=scroll|folder`).
- ARCH-GUI-003 measured 2026-10-26 on a desktop (Ryzen 7 7800X3D, WebKitGTK 2.54, 35 visible rows), not on the
  2020-class laptop the requirement names (`docs/02` §2 "M1 wave 6"): first screen 0.43 s after the process
  start (aligned and coloured 0.60 s); scrolling a 1,000,000-line comparison at 60.2 fps in all three phases, at
  the page's 60 Hz frame-clock ceiling; memory 598 MiB PSS for the GUI and its WebKit processes and 456 MiB RSS
  for the GUI process alone, against 400 MB = 381.5 MiB, so the memory limit is **not met** (an empty Home view
  already takes 168 MiB PSS). Reported to the maintainer; the toolkit is not switched (17-agent-workflow §7.1).
  Folder scan with CRC and *Show All*, every folder opened: 100,020 files per side in 0.26 s (UI-thread work
  ≤ 2.5 ms), 400,080 in 0.68 s with 7 rebuilds while scanning (≤ 8.9 ms), 1,000,200 in 1.64 s with 18 rebuilds
  (up to 21 ms with 1.1 million rows open, so frames are missed at that size).
- Wave-6 review fixes: Home no longer blocks the window on `-` (it read standard input on the UI thread);
  Stop while a folder comparison prepares stops it (no scan starts); the folder-view verdict and close code come
  from the core (`ViewOutcome::for_folder`, `PreparedFolderCompare::view_verdict`); a view whose `OpenView`
  arrives while quitting reports `shutdown` with 100 (`ViewOutcome::close_unopened`); the Text Compare status bar
  is visible again and the canvases are sized in whole device pixels (no per-frame rescaling); plain Enter, Left
  and Right reach focused buttons, lists and check boxes (the tree takes them while it has the focus); the
  next/previous-difference state is memoised per rebuild and selection; `gui list` shows the tab caption from the
  start; two empty files say so; the benchmark reports PSS and RSS in MiB against 381.5 MiB and the page frames
  while scanning; architecture check 7 no longer mistakes `&'static str` for a `static` item (contract §1.8.7
  item 37); `docs/reference/commands.md` lists `HomeCompare` and the Enter/Right/Left defaults.
- `docs/02-tech-stack.md`: the D-56 approval and the ARCH-GUI-003 measurements.

### Wave 5 — the `hikasa` command line
- `hikasa-cli`: the `hikasa` binary over the session service (contract §1.9): `quick`, `text compare`,
  `folder compare`, `format list|show|detect|tokens`, `vfs ls|stat|cat`, `gui open|ping|list|close|quit`,
  `--gui` on the compare commands, `completions` (bash, elvish, fish, powershell, zsh) and `version`; the global
  flags `--json`, `--quiet`, `--log`, `--verbose`, `--config-dir`, `--no-color`.
- Exit codes per CLI-004 and contract §2.21: usage errors and caught panics exit 100 (never 2 or 101), a broken
  pipe keeps the computed code; `gui open --wait` returns the view's code, 102 when it cannot wait, 103 without
  a GUI executable (D-01, D-02, D-03, D-22, D-23, D-35).
- One JSON document per invocation on stdout (also on error, `hikasa.usage_error.v1` for a command line that does
  not parse); progress and logs on stderr; `--output` reports written atomically (D-29, D-32, D-40).
- Vocabulary words are parsed by the core crates only; `gui open` uses the shared legacy parser (D-51, D-57).
- Integration tests for every subcommand with goldens for every JSON schema. The launch and wait paths run in
  the `cli_gui_launch` test target (`harness = false`), whose own binary is the scripted GUI when
  `HIKASA_TEST_FAKE_GUI=1`; no test GUI is built or installed as a program.
- Gates G1 and G2 through the binary (`cargo test --release -p hikasa-cli --test perf_gates -- --ignored`;
  not run in CI). Measured 2026-10-25 (Ryzen 7 7800X3D, release): G1 (2 × 1M lines, `--report summary`)
  0.43 s auto, 0.45 s standard, 0.36 s identical, peak RSS 298 MiB (bound 427 MiB; threshold 3 s); G2 (100k files
  per side, `--content none`) 0.05 s warm (threshold 1.5 s) and 0.694 s cold with dropped caches, measured
  2026-10-26 on btrfs over NVMe (threshold 5 s; `docs/12` §4).
- Wave-5 review fixes: `gui open -- -x a.c` reaches the shared parser with its `--` (D-51); `--json` counts
  only before `--`, a legacy `-json` selects the usage-error document when the `gui open` arguments do not
  parse, and that document keeps clap's whole message; `--version` prints `hikasa <version>` after a subcommand
  too (with `--json` the version document); a global value flag repeated with different values at different
  positions exits 100; `quick --crc|--binary` on an unreadable file reports the error on stderr and with
  `ok: false` in JSON; one error line per failed stdout write; the completions golden checks the field order.
- `hikasa-session`: a GUI executable that exists but cannot be started exits 103 (`ApiError::GuiNotStartable`,
  was 100 "ipc I/O"); `find_gui_exe` resolves a relative `HIKASA_GUI_EXE` against the current folder and skips
  candidates without an execute bit.
- `hikasa-vfs`: writing to a symbolic link replaces the file it points to and keeps the link; a FIFO or a
  device (`/dev/null`) is written to directly and never replaced (`--output`; contract §1.2.7 item 12).
- `docs/12-cli-and-scripting.md`: the M1 flag set, the wave-5 decisions and the measured gates; contract
  §1.9.3 (accepted additions and deviations of wave 5), §1.9.2 and §6.1 amended; `docs/11` §1 local writes.

### Wave 4 — reports and the session service
- `hikasa-report`: text reports (the annotated unified diff, normal, context and unified patches, side-by-side
  text and self-contained HTML, the summary, the statistics CSV) and folder reports (status list, side-by-side
  text and HTML, summary, the `hikasa.folder_compare.v1` JSON document); patches are byte-exact in every style
  (checked with GNU `patch` and `git apply`), with BOM and encoding warnings, escaping, grapheme-aware column
  widths and RPT-004 headers.
- `hikasa-session`: host capture, configuration and cache folders, options, `FormatStore` (built-ins, user
  formats, `order.toml`), the service API shared by the CLI and the GUI (requests, jobs, the panic boundary),
  IPC protocol v1 with framing, client and server, one endpoint per user and configuration, and the
  architecture test (checks 1–7).
- Review fixes (78 findings from break, conformance and consumer reviews; 71 fixed with tests, 7 rejected with
  reasons, contract §1.8.7): the IPC client verifies the socket folder's owner and permissions, dropped view
  waiters close as `abandoned`, shutdown releases waiters, writes never block the GUI, per-call deadlines,
  bounded frames and queues; `--config-dir` and the endpoint override reach a launched GUI (`HIKASA_GUI_EXE`
  for tests); a private, symlink-safe standard-input spool; pool panics never abort; the effective
  ignore-unimportant setting reaches folder JSON and reports; adapter API for the request ↔ view mapping, lazy
  standard input, vocabulary parsers, the D-23 close rules and escaping helpers.
- `hikasa-core`: `WarningCode::PatchEncoding`. Workspace at the end of the wave: 983 tests.

### Wave 3 — text and folder comparison
- `hikasa-text`: `TextDocument` (chunked, cancellable decoding; encodings, line endings, size limits), text
  rules, the compare engine (an identity fast path equal to the general path for every grammar, bounded Auto
  alignment, closeness pairing with work budgets), importance classification of differences, sections,
  statistics, display filters, the thumbnail, a progressive observer, and quick compare by rules that agrees
  with the full comparison (external conversions on every path).
- `hikasa-folder`: folder rules and criteria, a parallel cancellable scanner with name-only prefilter and
  re-stat, name alignment, the live `FolderModel` with O(1) counters, states and aggregates, display and folder
  display filters, flatten mode, symbolic link loop handling, content tests within a byte budget through
  injected comparators, and verdicts.
- Adversarial and conformance reviews per crate; accepted deviations in contract §1.5.7 and §1.6.8. Gates G1,
  G2 (warm), G4 and G5 pass. Workspace at the end of the wave: 724 tests.

### Wave 2 — file access, alignment and file formats
- `hikasa-vfs`: entry model, the `Vfs` trait, `LocalVfs`, `MemoryVfs` with fault injection, standard-input and
  clipboard blobs, the router, name-only prefilter and re-stat, streaming and memory-mapped CRC32 and binary
  comparison with scoped parallel chunks (results independent of the thread count). The memory-map module is
  the only `unsafe` code in the libraries.
- `hikasa-diff`: interner, the Unaligned, Standard, Myers, Histogram and Patience alignment engines, a bounded
  Standard fallback (`alignment_degraded`), closeness matching with cell budgets, never-align, inline
  differences and helper-thread cancellation.
- `hikasa-formats`: the format model and its TOML form, the grammar tokenizer (linear on long lines,
  cancellable inside a line), regular-expression engine selection, the registry with ordering and shadowing,
  21 original presets, encoding and binary detection, Sorted and External conversions.
- `hikasa-core`: `WarningCode::AlignmentDegraded`. Every crate had an adversarial and a conformance review;
  accepted deviations are recorded in the contract. Workspace at the end of the wave: 519 tests.

### Wave 1 — `hikasa-core`
- Sides, spec parsing and canonical form (local, standard input, clipboard, remote and HTTP schemes), error
  kinds and classification, cancellation tokens, progress sinks with per-phase throttling, job ids,
  `FileTime` (strict RFC 3339), case folding, the file-mask engine and `NameFilter`, the content comparator
  contract, warnings and exit codes (ARCH-020, VFS-010, FC-050, FC-051, FC-021, CLI-004).
- The architecture test (checks 1, 2, 4, 5), property suites, golden tables and the `mask_match` benchmark.
  Review: 22 findings from two reviewers, 20 fixed with regression tests, 1 rejected per contract §1.1.3.

### Preparation
- Workspace dependencies, lints (`unsafe_code` denied, `missing_docs`), MSRV 1.97 and the CI workflow (fmt,
  clippy and tests on Linux, Windows and macOS, cargo-deny).
- `docs/design/m1-contracts.md`: the binding M1 API contract (crate APIs, normative algorithms, JSON and IPC
  shapes, concurrency and error model, test plan, wave acceptance criteria, decision register).

## 0.0.0 — 2026-10-19
- Initial specification and workspace skeleton. No functionality.
- Restructured workspace into three layers (`core/`, `cli/`, `gui/`); CLI-first rule:
  every function is a `hikasa` subcommand, the GUI is reached via `hikasa gui` (IPC).
