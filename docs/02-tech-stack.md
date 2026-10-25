# Technology Stack and Crate Decisions

Language: **Rust** (stable toolchain, edition 2024, MSRV = current stable minus two
releases). Only open-source dependencies with licenses compatible with the project
license (MIT OR Apache-2.0) are allowed. GPL/LGPL crates are *not* allowed because
they would force relicensing of the binary; MPL-2.0 is acceptable (file-level
copyleft, no relicensing). Verify the license field on crates.io before adding any
dependency (`cargo deny check licenses` runs in CI). The allowed set lives in `deny.toml`: MIT,
Apache-2.0 (also with the LLVM exception), BSD-2/3-Clause, ISC, Zlib, CC0-1.0, Unlicense, MPL-2.0,
Unicode-3.0/DFS-2016, OpenSSL, BSL-1.0, 0BSD and NCSA (BSD-style; it arrives through the GUI toolkit's
image stack).

Project license: **MIT OR Apache-2.0** (dual), the Rust ecosystem convention. This
keeps every crate below compatible and lets the project itself be reused freely.

## 1. Proposed crates (reviewed)

The table reviews the initially proposed list and adds what is missing. "Decision"
is the binding choice for implementers; versions are the latest at the time of
writing and must be re-checked when the dependency is actually added.

| Area | Crate | License | Decision | Notes |
|------|-------|---------|----------|-------|
| Text diff core | `imara-diff` 0.2 | Apache-2.0 | **Adopt** for the Myers/Histogram engines | Fastest Rust LCS implementation (used by gitoxide). Token-based API lets us diff lines, words or characters with our own token interning. |
| Simplified diff / unified output | `similar` 3.x | Apache-2.0 | **Adopt** for unified/context patch text and for word/char inline diffs where convenient | Has Patience and Myers with deadline support, `TextDiff` helpers, unified diff formatter. |
| Patch parse/apply | `diffy` 0.5 | MIT OR Apache-2.0 | **Adopt** for parsing patch files and applying them (Text Patch view, Apply Patch) | Covers unified diffs; normal/context diffs need our own small parser (see `05-text-views.md`). |
| Folder scanning | `ignore` 0.4 | MIT OR Unlicense | **Adopt** with care | Gives parallel walking and gitignore-style globs. We must disable its hidden/ignore defaults and feed our own mask engine; we do not want `.gitignore` semantics by default (option later). For archives/remote back-ends our own walker is used. |
| Parallelism | `rayon` 1.12 | MIT OR Apache-2.0 | **Adopt** | CPU pool for alignment, hashing, pixel compare. |
| Hashing | `blake3` 1.8 | CC0/Apache-2.0 | **Adopt** for content hashing of large files | BLAKE3 is the internal "fast content fingerprint". **Also add `crc32fast`** (MIT/Apache): CRC32 is required because archives (zip) provide CRC32 for free and snapshots store CRC32 for compatibility with "CRC comparison" semantics. |
| File watching | `notify` 8 + `notify-debouncer-full` | CC0 / MIT OR Apache | **Adopt** | Powers "check for files changed on disk" and automatic refresh. |
| Encodings | `encoding_rs` 0.8 | Apache/MIT + BSD-3 | **Adopt** for decoding/encoding | Covers all WHATWG encodings (Windows-125x, ISO-8859-x, KOI8, Shift-JIS, EUC, GB18030, Big5, UTF-16). **Add `chardetng`** (Apache/MIT) for detection; BOM detection is our own. |
| Binary detection | `content_inspector` 0.2 | MIT/Apache | **Replace** | Unmaintained since 2018. Implement our own detector in `hikasa-formats` (BOM + NUL/control-byte heuristics, same as the crate but with UTF-16 awareness) and use `infer` (MIT) for magic-number based type sniffing (archives, images, audio). |
| Syntax highlighting | `syntect` 5 | MIT | **Adopt** as the *highlighting* back-end for languages we do not define grammars for, via `two-face` (MIT/Apache) syntax set | Our own grammar engine (`10-file-formats.md`) is authoritative for *importance* classification because it must be user-editable and simple. Syntect is optional eye-candy highlighting; the importance tokenizer must not depend on it. |
| Large text | `ropey` 1.6 | MIT OR Apache-2.0 | **Adopt** | Rope for editable panes; supports line indexing and efficient edits for 100 MB files. |
| Regex | `regex` 1 | MIT OR Apache-2.0 | **Adopt** as default engine | Linear-time, safe. **Add `fancy-regex`** (MIT) for user patterns that need backreferences (`\1`) or lookaround (the rename and grammar dialogs advertise backreferences); choose engine automatically per pattern. |
| Serialization | `serde` 1 + `toml` | MIT OR Apache-2.0 | **Adopt** | TOML for settings/sessions/formats; JSON (`serde_json`) for machine-readable reports and IPC. |
| Errors | `thiserror` 2 (libs), `anyhow` (bins) | MIT OR Apache-2.0 | **Adopt** | |
| Async | `tokio` 1 | MIT | **Adopt** for I/O-bound work (remote back-ends, IPC, watchers) | Engines stay synchronous. |

### Additional crates required by the feature set

| Area | Crate | License | Purpose |
|------|-------|---------|---------|
| Globs/masks | `globset` | MIT OR Unlicense | Base for file masks (`*.c;-*.bak;Source\`), extended by our mask semantics. |
| Archives | `zip` (MIT), `tar` (MIT/Apache), `flate2` (MIT/Apache), `bzip2` (MIT/Apache), `xz2` or `liblzma` (MIT/Apache), `zstd` (BSD-3), `sevenz-rust` (Apache-2.0) | — | Read/write zip, tar(.gz/.bz2/.xz/.zst), 7z. RAR: `unrar` crate is MIT/Apache but links the proprietary-licensed UnRAR library → **optional feature, off by default**, documented caveat. Formats without a pure-Rust reader (cab, rpm, deb = ar+tar, iso, wim, chm) are added incrementally; deb/rpm/ar are easy, iso via `iso9660` crates, cab/wim/chm are P2. |
| Spreadsheets | `calamine` 0.37 | MIT | Read xls/xlsx/xlsm/ods for Table Compare (all sheets). Writing back is P2 (`rust_xlsxwriter`, MIT/Apache). |
| CSV | `csv` 1 | MIT OR Unlicense | Delimited parsing (we need our own tokenizer for "consecutive delimiters as one" and fixed-width; `csv` used for the standard path). |
| Office text extraction | `docx-rs` (MIT), `quick-xml` (MIT), `zip` | — | Extract text from docx/odt/odp/ods (zip + XML). Legacy binary `.doc`/`.xls` need external converters (configurable "external program" conversion). |
| RTF | `rtf-parser` | MIT | Text extraction from RTF. |
| PDF | `pdf-extract` | MIT | Text extraction from PDF (P1). |
| Images | `image` 0.25 | MIT OR Apache-2.0 | Decode PNG/JPEG/GIF/BMP/TIFF/ICO/WebP/TGA/PNM/EXR/QOI; `kamadak-exif` (BSD-2) for EXIF metadata; `resvg` (MIT/Apache) for SVG rasterization (P2). |
| Audio tags | `lofty` 0.25 | MIT OR Apache-2.0 | ID3v1/v2, MP4 ilst, FLAC Vorbis comments, APE, RIFF INFO → Media Compare. |
| Executable version info | `pelite` (MIT) or `goblin` (MIT) | — | PE `VS_VERSIONINFO`, header flags; `goblin` also parses ELF/Mach-O for a cross-platform "Version Compare" (build-id, soname, arch). |
| FTP/FTPS | `suppaftp` 12 (with `native-tls`/`rustls` feature) | MIT OR Apache-2.0 | FTP client incl. TLS, MLSD, passive/active. |
| SFTP | `russh` + `russh-sftp` | Apache-2.0 | SSH client with agent and key-file auth. |
| WebDAV / HTTP | `reqwest` 0.13 + `reqwest-dav` | MIT OR Apache-2.0 | WebDAV PROPFIND/GET/PUT, HTTP single-file specs, OAuth flows for cloud drives. |
| S3-compatible | `rust-s3` (MIT) or `aws-sdk-s3` (Apache-2.0) | — | Prefer `rust-s3` for small footprint and non-AWS endpoints; AWS credential-file/env parsing implemented per spec `11-vfs-remote-archives.md`. |
| Credentials | `keyring` 4 | MIT OR Apache-2.0 | Store saved passwords/tokens in the OS keychain (Secret Service, Keychain, Credential Manager). |
| Time | `jiff` (Unlicense/MIT) or `time` | — | Timestamps with time-zone tolerance logic, DST handling. |
| Filesystem extras | `filetime` (set mtime), `trash` (recycle bin), `memmap2` (mmap), `same-file`, `fs-err` | MIT/Apache | |
| Unicode | `unicode-normalization`, `unicode-width`, `unicode-segmentation` | MIT/Apache | NFC/NFD filename alignment, column widths, grapheme-aware cursor. |
| Clipboard / dialogs / open | `arboard` (MIT/Apache), `rfd` (MIT), `open` (MIT) | — | Clipboard specs, native file dialogs, "open with associated application". |
| CLI | `clap` 4 (+ `clap_complete`) | MIT OR Apache-2.0 | Subcommand tree, shell completions; accepts `/switch` (Windows style) and `--switch` forms for legacy options. |
| IPC | `interprocess` (MIT/Apache) + `serde_json` | — | Local socket / named pipe between `hikasa` and `hikasa-gui`. |
| Logging | `tracing` + `tracing-subscriber` | MIT | Structured logs; Log panel subscribes to a tracing layer. |
| Config dirs | `directories` 6 | MIT OR Apache-2.0 | Per-platform config/cache paths. |
| Property testing | `proptest`, `criterion` (dev) | MIT/Apache | |
| License audit | `cargo-deny` (CI tool) | — | Enforces the license policy. |

### M1 dependency constraints (binding for milestone M1)

Milestone M1 builds only with the crates listed in `[workspace.dependencies]` of the root `Cargo.toml`
(`docs/design/m1-contracts.md` §0.5 assigns them to crates). Several crates of the tables above are therefore
**not available in M1**; the consequences below are part of the M1 design, and adding any of these crates
needs the approval process of `17-agent-workflow.md` §2.6.

| Missing crate | Consequence in M1 |
|---|---|
| `tokio` | IPC is blocking: one accept thread plus one thread per connection (ARCH-080's tokio runtime is deferred). |
| `ignore`, `globset` | In-house folder walker (`hikasa-folder` scanner) and mask engine (`hikasa_core::mask`, `04-folder-views.md` §6.1). |
| `notify` | No file watching: FC-017 automatic refresh, TC-006 and SES-053 change detection are M2+. |
| `arboard` | `clipboard://` resolves only when an adapter injects the clipboard content; the CLI reports "unsupported"; `gui open clipboard://` is rejected. |
| `infer` | Magic-number detection is an in-house table (`hikasa-formats`). |
| `libc`, `thread-priority` | ARCH-061 comparison priority cannot be applied (a documented gap); owner and group are shown as numeric ids; the effective uid is obtained with std only. |
| a signal-handling crate | Ctrl-C terminates the CLI (no graceful cancel); every `--output` file is written atomically, so no partial report appears. |
| `dioxus` | *Approved for M1 wave 6 (D-56):* `dioxus` 0.7 with the `desktop` feature only, used by `hikasa-gui` alone (§2, "M1 wave 6"). Its WebView stack (wry; WebKitGTK on Linux) brings the NCSA licence of the allow list. |
| `tempfile` (dev-only) | Atomic writes in library code are hand-rolled (temp file + fsync + rename). |

Also inside the approved list, M1 code does not use `blake3` (content caches are P1; IPC endpoint hashing
uses an in-house FNV-1a), and uses `similar` and `diffy` only as test oracles (one LCS implementation,
`imara-diff`, keeps behaviour single-sourced; patch apply is P1).

**Feature notes.** No workspace-level feature is changed for M1; the default feature sets shape the design:

- **`toml`** (1.x) is used without `preserve_order`: every table deserialises through a `BTreeMap`, so no code
  may depend on the key order of a TOML table (arrays keep their order). Grammar element ids follow the order
  of first use in the `grammar` array, not the `[elements]` table.
- **`serde_json`** is used without `preserve_order`: JSON produced from structs keeps declaration order, while
  `serde_json::Value` objects sort their keys.
- **`interprocess`** (2.x) is used with its default features only (no `tokio`/`async`): the IPC client and
  server use blocking local sockets and named pipes; timeouts use the stream's receive timeout.
- **`chardetng`** (1.0) exposes `find_score` only behind a testing-only feature, so a format's fallback
  encoding applies only to content without non-ASCII evidence.
- **`jiff`** has no `serde` feature enabled: timestamps (`hikasa_core::FileTime`) have hand-written serde
  implementations (RFC 3339 UTC text for the years 0000–9999 that RFC 3339 can write, `@<secs>.<nanos>`
  otherwise; a native TOML offset date-time is accepted when reading). jiff is not part of `hikasa-core`'s
  public API.
- **`ropey`** keeps its default Unicode line breaks, so Hikasa keeps its own line table (lines end only at CR,
  LF or CRLF).

**mmap residual risk.** Mapping a file with `memmap2` is `unsafe`, because another process may modify or
truncate a file while it is mapped; it is the only `unsafe` code in Hikasa's libraries and programs
(`core/hikasa-vfs/src/mmap.rs`, ARCH-005). Mitigations: the map is read-only and private to that module; it is
created only for local regular files confirmed after opening; a mapping covers one 8 MiB chunk and lives only
while that chunk is compared or hashed, inside one call (the optional helper threads of a call are scoped and
end before it returns); on Linux each chunk is pre-faulted with `MADV_POPULATE_READ` (kernel 5.14 or later)
before it is read, which reports a chunk cut off by a truncation as an error instead of raising SIGBUS, and the
comparison or CRC then falls back to streaming; and after the call both files are re-stat'ed, so a changed
length or modification time discards the mapped result in favour of streaming (`11-vfs-remote-archives.md` §3).
**Accepted residual risk:** a concurrent truncation of an input file between pre-faulting a chunk and reading
it — or at any time while a chunk is mapped on other systems and on Linux kernels older than 5.14 — can
terminate the process (SIGBUS on Unix). Setting the mmap policy to "never" disables the mapped path.

**Test-only `unsafe`.** Every library crate other than `hikasa-vfs` is `#![forbid(unsafe_code)]`. The only other
`unsafe` in the repository is in allocation tests and benches: a file in a crate's own `tests/` or `benches/`
directory that installs a counting `#[global_allocator]` needs `unsafe`, because `GlobalAlloc` is an `unsafe`
trait. Such a file says `#![allow(unsafe_code)]` (the workspace lint is `deny`) and has a `// SAFETY:` comment
before each `unsafe`. The architecture test allows `unsafe` in exactly these files and in `mmap.rs`, and nowhere
else, not in any library module (`src/tests/` included); see `design/m1-contracts.md` §1.2.6 and §6.1 check 1.

## 2. GUI toolkit decision

Requirements that drive the choice:

1. Custom, very high-performance widgets: side-by-side text editors with per-character
   colouring on million-line documents, a grid with millions of rows, hex panes,
   thumbnails, pixel-diff canvases. Rendering must be *virtualized* (only visible rows
   are laid out) and must not allocate a DOM node per line.
2. Standard desktop chrome: menus, toolbars, tabs that can be dragged between
   windows, modal dialogs, native file dialogs, system theme (light/dark), HiDPI.
3. Cross-platform Linux/Windows/macOS from one code base, permissive license.
4. Productivity for AI agents: declarative UI with hot reload is a plus.

| Toolkit | License | Pros | Cons |
|---------|---------|------|------|
| **Dioxus 0.7 (desktop)** | MIT/Apache | Declarative RSX, hot reload, signals, mature desktop target (WebView via `wry`), CSS styling, large ecosystem familiarity (React-like) | Large virtualized text/grid widgets must be built on `<canvas>`/custom components to stay fast; WebView memory overhead; native menus need platform glue (`muda`). |
| egui 0.37 | MIT/Apache | Immediate mode, trivially virtualized custom painting, excellent for editors/grids, GPU via wgpu, single code path | Non-native look, accessibility weaker (improving via AccessKit), layout of complex dialogs is more manual. |
| iced 0.14 | MIT | Elm architecture, wgpu renderer, good custom widgets | Smaller ecosystem, slower iteration, fewer ready widgets. |
| Slint 1.18 | GPL-3.0 or commercial | Polished | GPL forces relicensing → **rejected** by policy. |
| GTK4/Relm4, Qt bindings | LGPL/GPL | Native | Linking/licensing constraints, heavy toolchain → rejected. |

**Decision (binding, with a measurable gate):**

- **ARCH-GUI-001** (P0): Use **Dioxus 0.7 desktop** for application chrome and
  dialogs, as proposed.
- **ARCH-GUI-002** (P0): Implement the four heavy views (text panes, table grid,
  hex panes, thumbnail/picture canvas) as *custom virtualized components*: a Rust-side
  layout model computes only the visible slice; rendering goes to an HTML canvas
  (2D context, text run batching) owned by the component. No per-line DOM nodes.
- **ARCH-GUI-003** (P0): Milestone 1 contains a *performance gate*: scrolling a
  side-by-side comparison of two 1,000,000-line files with syntax colouring must hold
  ≥ 60 fps with < 400 MB RSS on a 2020-class laptop, and opening must show the first
  screen in < 1 s. If the Dioxus/WebView path fails the gate after reasonable
  optimization, the project switches those views to **egui** embedded in a native
  window (keeping Dioxus only if a hybrid is practical, otherwise moving the whole UI
  to egui). The engine crates are unaffected by this decision by design (ARCH-012).
- **ARCH-GUI-004** (P1): Native menus and global shortcuts via `muda`
  (MIT/Apache) and `global-hotkey`; tray icon via `tray-icon` for the optional
  clipboard-history helper.

### M1 wave 6: approval (D-56) and the ARCH-GUI-003 measurements

**Approval.** The maintainer approved `dioxus` 0.7 with the `desktop` feature for wave 6 (D-56); it is in
`[workspace.dependencies]` and used by `hikasa-gui` only (contract §0.5). On Linux it runs on wry and
WebKitGTK 4.1 (with GTK 3 and libsoup 3); the M1 window has HTML menus generated from the command catalogue
(ARCH-GUI-004's native menus stay P1).

**How the heavy view is built (ARCH-GUI-002).** The text panes are one `<canvas>` per view. The page asks the
Rust side for the rows it is about to draw (`/hk/slice` on the webview's own protocol); Rust lays out only those
rows from the `TextComparison` (line numbers, row states, syntax spans, inline differences) and answers with a
compact JSON slice (0.2–0.5 ms per request). The page keeps a bounded row cache, requests the rows that are
missing in one request, prefetches the next pages while scrolling and the next frame's rows while the scroll bar
is dragged, draws text runs batched per pane and colour, and moves the drawn image when only the scroll position
changed (only the uncovered rows are drawn). No DOM node exists per line. The folder tree is a DOM list that
only holds the rows around the scroll position.

**Measurement** (`cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture
--test-threads 1`, which runs the binary in its scripted mode `HIKASA_GUI_BENCH=scroll|folder` and prints a JSON
summary; re-measured 2026-10-26 after the wave-6 review fixes). **Machine:** a desktop, not the "2020-class
laptop" that ARCH-GUI-003 names — AMD Ryzen 7 7800X3D (8 cores, 16 threads), 64 GB RAM, Radeon RX 7900, one
3840 × 2160 display at 120 Hz, KDE Plasma on Wayland with the window on Xwayland (`GDK_BACKEND=x11`),
WebKitGTK 2.54, device pixel ratio 1.5, text area 1254 × 650 CSS px = 35 rows at the 13 px default font. The
figures are therefore **not verified on the reference hardware**. Input: two generated 1,000,000-line C files
(≈ 48 bytes per line, about 1 % of the lines changed in clusters; 1,003,667 rows; format "C, C++, C#, ObjC
Source", syntax colouring on).

Memory is reported in MiB (1 MiB = 1,048,576 bytes) against the gate of 400 MB = **381.5 MiB**. The figure for
the whole window is the sum of the *proportional* set sizes (PSS, `/proc/<pid>/smaps_rollup`) of the GUI process
and its WebKit processes: pages the processes share (the WebKit and GTK libraries, shared buffers) count once.
Summed RSS (which counts them once per process) and the GUI process's own RSS are given alongside.

| ARCH-GUI-003 item | Measured | Gate |
|---|---|---|
| First screen (the loaded text, before the alignment) | 0.43 s after the process start (0.26 s after the view opened) | < 1 s: **met** |
| Aligned comparison with colours on screen | 0.60 s after the process start (comparison done 0.41 s after the view opened) | — |
| Wheel-like scrolling, 3 lines per frame, 600 frames | 60.2 fps; frame work p50 4 / p95 4 / max 7 ms; no frame interval above 25 ms | ≥ 60 fps: met at the page's 60 Hz frame-clock ceiling, not verified on 2020-class hardware |
| A page per frame, 300 frames | 60.2 fps; frame work p95 3 ms; no interval above 25 ms | as above |
| Sweep through the whole comparison in 1,200 frames (836 rows per frame) | 60.2 fps; frame work p50 4 / p95 5 / max 43 ms; 3 intervals above 25 ms | as above |
| Memory after scrolling | **PSS of the GUI and its web processes 598 MiB** (GUI 356, web content 214, network 28); GUI process RSS alone **456 MiB** (peak 465); summed RSS 821 MiB | < 381.5 MiB: **not met** (both the total and the GUI process alone) |

The page's frame clock runs at 60 Hz here (WebKitGTK drives `requestAnimationFrame` at 60 Hz on X11 although the
display refreshes at 120 Hz), so about 60 fps is the most this setup can show; the values above are as measured,
not rounded up. The wave-6 review run of the previous build gave 59.5 fps with 5 intervals above 25 ms in the
wheel phase: its canvas buffer was not a whole number of device pixels, so the compositor rescaled it on every
frame. The canvases are now sized in whole device pixels; without that, a correctly laid-out text view (status
bar visible) scrolled at 25–29 fps on this machine.

The memory figure does not meet the gate. With an empty Home view the window already takes 168 MiB PSS (GUI
process 72, web content 67, network 29; summed RSS 366 MiB, GUI process RSS 160 MiB); the 1,000,000-line
comparison adds about 285 MiB PSS (295 MiB RSS) to the GUI process (both documents, the rows, the tokenizer start states and the
allocator's retained peak; the CLI's G1 peak for the same work is 298 MiB) and about 150 MiB PSS to the web content
process (canvas buffers, the bounded row cache, script heap). Even the GUI process alone (456 MiB RSS) is above
the limit.

Acceptance (b), the folder scan (same machine, same mode; CRC content tests, display filter *Show All*, every
folder opened as it arrives while the page scrolls the tree; warm page cache, since dropping it needs root):

| Tree (files per side) | Scan | Rebuilds while scanning | Rows at the end | Tree rebuild | UI-thread work per message batch | Page frame intervals while scanning | Memory (PSS total / GUI RSS) |
|---|---|---|---|---|---|---|---|
| 100,020 in 11,110 folders | 0.26 s | 2 | 111,150 | max 2.5 ms | max 2.5 ms | 7 frames, p95 28 ms, 1 above 25 ms | 237 / 217 MiB |
| 400,080 in 44,440 folders (`HIKASA_PERF_TREE_TOP=40`) | 0.68 s | 7 | 444,600 | p50 5.9 / max 8.9 ms | p95 6.4 / max 8.9 ms | 32 frames, p95 17 ms, 1 above 25 ms | 311 / 287 MiB |
| 1,000,200 in 111,100 folders (`HIKASA_PERF_TREE_TOP=100`) | 1.64 s | 18 | 1,111,500 | p50 11.3 / max 21.3 ms (7 of 19 above 16.7 ms) | p95 19.5 / max 21.3 ms (7 of 83 above 16.7 ms) | 90 frames, p95 23 ms, 1 above 25 ms | 442 / 417 MiB |

Each root render took at most 0.9 ms. The tree rendering is incremental (the visible rows are rebuilt from the
live model at most every 66 ms while the scan runs), and the UI-thread work stays below 16 ms per frame for the
100k-file tree that acceptance (b) names and for 400k files. The rebuild walks every visible row, so with about a
million rows open (everything expanded under *Show All*) it takes up to 21 ms and frames are then missed; the page
frame intervals measure the WebView's frames, not only the UI-thread work. A cold scan was not measured.

**Decision status (17-agent-workflow §7.1).** The first screen and the scrolling items pass on this desktop (at
the frame-clock ceiling, not on the reference laptop); the memory limit is **not met**: 598 MiB PSS for the window,
456 MiB RSS for the GUI process alone, against 381.5 MiB, because of the WebView's own memory plus the
comparison's data. The toolkit is *not* switched: the measurements are reported to the maintainer, who decides
between accepting a higher memory budget for the WebView path, a hybrid with an embedded native text view, or the
egui fallback of ARCH-GUI-003.

## 3. Crates explicitly rejected

| Crate | Reason |
|-------|--------|
| `content_inspector` | Unmaintained; replaced by in-house detector + `infer`. |
| `slint` | GPL-3.0 / commercial. |
| `unrar` (default) | Links non-OSI UnRAR source; opt-in feature only. |
| `pcre2` | C library with build complexity; `regex` + `fancy-regex` cover the needed subset. |
| `walkdir` alone | `ignore` provides a parallel walker on top of it; keep `walkdir` only transitively. |

## 4. Build and tooling

- `cargo fmt`, `cargo clippy -D warnings`, `cargo test`, `cargo deny`, `cargo
  nextest` in CI on Linux, Windows, macOS.
- Release builds with LTO, `codegen-units=1`, `panic=unwind` (we must survive
  panics in a job and report them).
- Packaging: AppImage/Flatpak (Linux), MSI/MSIX (Windows), DMG (macOS) — P1.
