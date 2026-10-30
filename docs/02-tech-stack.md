# Technology Stack and Crate Decisions

Language: **Rust** (stable toolchain, edition 2024, MSRV = current stable minus two
releases). Only open-source dependencies with licenses compatible with the project
license (MIT OR Apache-2.0) are allowed. GPL/LGPL crates are *not* allowed because
they would force relicensing of the binary; MPL-2.0 is acceptable (file-level
copyleft, no relicensing). Verify the license field on crates.io before adding any
dependency (`cargo deny check licenses` runs in CI). The allowed set lives in `deny.toml`: MIT,
Apache-2.0 (also with the LLVM exception), BSD-2/3-Clause, ISC, Zlib, CC0-1.0, Unlicense, MPL-2.0,
Unicode-3.0/DFS-2016, OpenSSL, BSL-1.0 and 0BSD; the free font licences OFL-1.1 and Ubuntu-font-1.0 are accepted
only for the fonts egui embeds (`epaint_default_fonts`).

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
| `arboard` | M2 W3 (D-100, D-101): `clipboard://` works in the CLI and the window; see the M2 table below. |
| `infer` | Magic-number detection is an in-house table (`hikasa-formats`). |
| `libc`, `thread-priority` | ARCH-061 comparison priority cannot be applied (a documented gap); owner and group are shown as numeric ids; the effective uid is obtained with std only. |
| a signal-handling crate | Ctrl-C terminates the CLI (no graceful cancel); every `--output` file is written atomically, so no partial report appears. |
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

### M2 additions (D-63; each crate enters in the wave that first uses it, with fixed features)

| Crate | Wave | Features | Use |
|---|---|---|---|
| `trash` 5 (MIT) | W3 | `default-features = false`, `chrono`, `coinit_apartmentthreaded` (`chrono` writes the freedesktop `DeletionDate`; apartment-threaded COM on Windows). New transitive crates: `chrono`, `iana-time-zone`, `urlencoding` (Linux and the BSDs), `windows` (Windows), `objc2-foundation` (macOS) | the recycle bin of `LocalVfs` (`rm --trash`, FC-073, D-98) in `hikasa-vfs`, called only from `local/trash.rs`; it reads the home and data folders and the clock inside the call (an OS side effect, not a result input, T1) and never falls back to a permanent delete |
| `arboard` 3 (MIT OR Apache-2.0) | W3 | `default-features = false`, `image-data`, `wayland-data-control` (the set `Cargo.lock` already compiled through `egui-winit`; no new crate) | the system clipboard behind `clipboard://` (D-100) in `hikasa-session`, named only in `host.rs` (`SystemClipboard`, adapter-only); images become an in-house uncompressed PNG until W12 |
| `zip` 9 (MIT) | W2 | `default-features = false`, `deflate` (the pure-Rust back-ends `zlib-rs` via `flate2`, and `zopfli`; no AES, no bzip2/lzma/zstd/time) | settings packages (`.hikasa-pkg`, D-86) in `hikasa-session`; W5 adds the archive features it needs |

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
| **egui 0.37 + eframe (chosen, D-56 as amended)** | MIT/Apache | Immediate mode, trivially virtualized custom painting, excellent for editors/grids, GPU via wgpu, single code path, no WebView process | Non-native look, accessibility weaker (improving via AccessKit), layout of complex dialogs is more manual. |
| Dioxus 0.7 (desktop) | MIT/Apache | Declarative RSX, hot reload, signals, mature desktop target (WebView via `wry`), CSS styling, large ecosystem familiarity (React-like) | Large virtualized text/grid widgets must be built on `<canvas>`/custom components to stay fast; WebView memory overhead (measured in wave 6: it failed the ARCH-GUI-003 memory limit); native menus need platform glue (`muda`). |
| iced 0.14 | MIT | Elm architecture, wgpu renderer, good custom widgets | Smaller ecosystem, slower iteration, fewer ready widgets. |
| Slint 1.18 | GPL-3.0 or commercial | Polished | GPL forces relicensing → **rejected** by policy. |
| GTK4/Relm4, Qt bindings | LGPL/GPL | Native | Linking/licensing constraints, heavy toolchain → rejected. |

**Decision (binding, with a measurable gate):**

- **ARCH-GUI-001** (P0): Use **egui 0.37 with eframe** (native window, immediate mode, GPU-drawn through
  wgpu) for the whole application: chrome, dialogs and the heavy views. *History:* Dioxus 0.7 desktop was the
  first choice and was built as the wave-6 prototype; it failed the ARCH-GUI-003 memory limit, and the maintainer
  moved the whole GUI to egui (the fallback this decision always named; see "M1 wave 6" below).
- **ARCH-GUI-002** (P0): Implement the four heavy views (text panes, table grid,
  hex panes, thumbnail/picture canvas) as *custom virtualized components*: a Rust-side
  layout model computes only the visible slice; rendering goes through egui's painter (text runs laid out per
  visible row, rectangles for backgrounds and highlights), with the scroll position kept as a row index so
  million-row views scroll exactly. No per-line widget or allocation for rows off screen.
- **ARCH-GUI-003** (P0): Milestone 1 contains a *performance gate*: scrolling a
  side-by-side comparison of two 1,000,000-line files with syntax colouring must hold
  ≥ 60 fps with < 400 MB RSS on a 2020-class laptop, and opening must show the first
  screen in < 1 s. The engine crates are unaffected by the toolkit choice by design (ARCH-012).
- **ARCH-GUI-004** (P1): The menu bar, toolbar, context menus and shortcuts are drawn by egui inside the window
  and generated from the command catalogue (ARCH-070), the same on every platform; an OS-level menu bar (macOS
  global menu via `muda`), global shortcuts (`global-hotkey`) and the tray icon for the optional
  clipboard-history helper (`tray-icon`) stay optional additions for later milestones.

### M1 wave 6: the toolkit decision (D-56) and the ARCH-GUI-003 measurements

**First prototype: Dioxus (wave 6).** The maintainer first approved `dioxus` 0.7 with the `desktop` feature
(D-56). The prototype drew the text panes on one `<canvas>` per view from slices the Rust side laid out, and the
folder tree as a DOM list of the rows on screen. Measured on 2026-10-26 (same desktop as below, WebKitGTK 2.54 on
Xwayland, 35 visible rows): first screen 0.43 s after the process start (coloured 0.60 s); scrolling a
1,000,000-line comparison at 60.2 fps in all three phases, at the WebView's 60 Hz frame-clock ceiling; but
**598 MiB PSS** for the GUI and its WebKit processes and **456 MiB RSS** for the GUI process alone (an empty Home
view already 168 MiB PSS), against 400 MB = 381.5 MiB: the memory limit was **not met**. The folder scan stayed
below 16 ms of UI work up to 400,000 files per side.

**Decision (maintainer, after the report).** The whole GUI moved to **egui 0.37 with eframe** (D-56 as amended,
the fallback ARCH-GUI-003 names); `[workspace.dependencies]` has `eframe` and `egui` with their default features
(winit with X11 and Wayland, the wgpu renderer, AccessKit, the default and system fonts) and no longer has
`dioxus`. The view models, the command catalogue, the IPC handler and every D-23 rule were kept; only the
rendering and event layer was rewritten (wave 6b).

**How the window is built (ARCH-GUI-001/002).** One eframe window: menu bar, tabs, toolbar and status bar are
generated from the command catalogue every frame (immediate mode); the window repaints on demand — the mailbox
the IPC handler and the jobs post to calls `Context::request_repaint`, and the window polls running jobs at
10 Hz besides. The text panes are custom-painted with egui's painter for the visible rows only: the view model
builds the slice on screen (line numbers, row states, syntax spans, inline differences; 0.3 ms for 52 rows), each
row side becomes one layout job with a section per colour (egui keeps the laid-out rows of the last frame, so a
small scroll lays out only the new rows), and backgrounds, highlights, gap bars, the thumbnail and the scroll bar
are rectangles. The UTF-16 column → byte/char table of a row side is built in one pass, so painting a row is
linear in its length (wave-6b review: a per-lookup scan made 4,096-column non-ASCII rows quadratic; the scripted
scroll of 200 such lines now ends in about 80 s, where it did not end within 600 s; the ASCII pair took about
2 minutes in the review). That 80 s run was started with `xvfb-run` but, with `WAYLAND_DISPLAY` still set, ran on
the real display (Radeon RX 7900 XTX on Vulkan, 120 Hz, 1.5 points per pixel), not under Xvfb; window tests and
measurements under Xvfb need `env -u WAYLAND_DISPLAY xvfb-run -a …` (17-agent-workflow §3.5). The scroll position is a row index kept as `f64`, not a pixel offset (an
`f32` pixel offset does not resolve single rows of a million-row view). The folder tree is painted the same way from
`FolderViewModel::window` (the rows on screen).

**Measurement** (`cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture
--test-threads 1`, which runs the binary in its scripted mode `HIKASA_GUI_BENCH=scroll|folder`: the window drives
the scrolling itself, one step per frame, and prints a JSON summary; 2026-10-26). **Machine:** a desktop, not the
"2020-class laptop" that ARCH-GUI-003 names — AMD Ryzen 7 7800X3D (8 cores, 16 threads), 64 GB RAM, Radeon RX
7900 XTX (wgpu on Vulkan, RADV), one 3840 × 2160 display at 120 Hz, KDE Plasma on Wayland (native Wayland
window), 1.5 points per pixel, window 1920 × 1080 points, text area 52 rows at the 13 px default font. Other
builds ran on the machine during the runs (load average 10–12). The figures are therefore **not verified on the
reference hardware**. Input: the same two generated 1,000,000-line C files as the prototype (1,003,667 rows,
format "C, C++, C#, ObjC Source", syntax colouring on).

Memory is in MiB (1 MiB = 1,048,576 bytes) against 400 MB = **381.5 MiB**: the PSS (pages shared with other
processes, such as the graphics driver, count with their share) and the RSS of the GUI process, both from one
`/proc/<pid>/smaps_rollup` snapshot (exact), and the peak RSS from `VmHWM` in `/proc/<pid>/status` (the only peak
the kernel keeps; maintained from per-CPU batched counters, so approximate and never compared with the smaps
figures); there are no other processes now. "Frame work" is the UI thread's time per frame from the start of the
frame (messages, job polling, tree rebuilds, layout and painting commands); "eframe CPU" is eframe's own CPU time
per frame (including tessellation and the GPU submission); "interval" is the time between frame starts.

| ARCH-GUI-003 item | Measured | Gate |
|---|---|---|
| First screen (the loaded text, before the alignment) | 0.51 s after the process start (0.32 s after the view opened; the window's first frame came at 0.46 s) | < 1 s: **met** |
| Aligned comparison with colours on screen | 0.70 s after the process start (comparison done 0.49 s after the view opened) | — |
| Wheel-like scrolling, 3 lines per frame, 600 frames | **119.8 fps** (the display's 120 Hz); frame work p50 0.34 / p95 0.63 / max 0.78 ms; eframe CPU p95 1.0 ms; intervals p50 8.33 / p95 8.42 / max 9.2 ms | ≥ 60 fps: **met** |
| A page per frame (51 rows), 300 frames | **119.6 fps**; frame work p50 2.2 / p95 2.7 / max 4.7 ms; eframe CPU p95 3.1 ms; intervals p95 8.41 / max 10.1 ms | **met** |
| Sweep through the whole comparison in 1,200 frames (836 rows per frame) | **119.9 fps**; frame work p50 2.4 / p95 3.4 / max 5.0 ms; eframe CPU p95 4.0 ms; intervals p95 8.44 / max 10.8 ms; no frame above 16.7 ms in any phase | **met** |
| Memory after scrolling | **PSS 275 MiB**, **RSS 281 MiB** (peak RSS during the comparison ≈ 365 MiB); an empty Home window takes 76 MiB PSS / 87 MiB RSS (first run, RSS then from `VmRSS`) | < 381.5 MiB: **met** (also the peak) |

egui runs at the display's refresh rate (vsync), so 120 fps is the ceiling here; every frame's work stays below
5 ms, a quarter of a 60 Hz frame. A dry run under Xvfb with Mesa's software renderer (llvmpipe, no GPU) reached
18–25 fps: without a GPU the rasterisation, not the Hikasa code, is the limit.

Acceptance (b), the folder scan (same machine and mode; CRC content tests, display filter *Show All*, every
folder opened as it arrives while the window scrolls the tree two rows per frame; the window's first frame,
which builds egui's font atlas in 28–78 ms, is reported apart and not counted):

| Tree (files per side) | Scan | Rebuilds while scanning | Rows at the end | Tree rebuild | UI work per frame while scanning | Frame intervals while scanning | Memory (PSS / RSS, RSS from `VmRSS`) |
|---|---|---|---|---|---|---|---|
| 100,020 in 11,110 folders | 0.21 s | 3 | 111,150 | max 2.8 ms | 15 frames: p50 0.36 / p95 2.3 / max 3.3 ms | p95 8.8 / max 8.9 ms | 105 / 115 MiB |
| 400,080 in 44,440 folders, warm cache (two runs) | 0.62 / 1.44 s | 8 / 17 | 444,600 | p50 5.0–6.0 / max 8.8–11.0 ms | 65 / 143 frames: p95 5.5–6.4 / max 9.2–11.8 ms, none above 16.7 ms | p95 9.1–9.2 / max 10.8–14.7 ms | 173 / 184 MiB |
| 400,080, scanned right after the tree was written (`HIKASA_PERF_TREE_TOP=40`) | 2.22 s | 27 | 444,600 | p50 8.2 / max 26.9 ms | 235 frames: p95 9.8 / max 28.5 ms, 2 above 16.7 ms | p95 13.9 / max 27 ms besides one 90 ms gap at the start | 143 / 184 MiB |

The tree renders incrementally (the visible rows are rebuilt from the live model at most every 66 ms while the
scan runs), and the UI work stays below 16 ms per frame for the 100k-file tree that acceptance (b) names and for
the 400k-file tree with a warm cache. The scan right after generating 800,000 files ran while the system was
still writing them back; the slower scan and the two longer rebuilds there come from that I/O (the release CLI
compares the same warm tree in 0.55 s). The rebuild walks every visible row, so with hundreds of thousands of rows
open it is the largest per-frame cost.

**Verdict (17-agent-workflow §7.1).** On this desktop the egui GUI **meets all three ARCH-GUI-003 items**: first
screen 0.51 s, ≥ 60 fps in every phase (119.6–119.9 fps at the 120 Hz display, every frame below 5 ms of UI work),
and 275 MiB PSS / 281 MiB RSS (peak ≈ 365 MiB) against 381.5 MiB. Not yet verified on a 2020-class laptop; the frame
rate has a wide margin (≤ 5 ms of UI work per frame), while the memory peak of a 1,000,000-line comparison (≈ 16 MiB
below the limit) is the item to re-check there.

**Memory re-measured after the wave-6b review.** The first run took the RSS and the peak from `VmRSS`/`VmHWM` and
the PSS from `smaps_rollup`; `VmRSS` is an approximate counter, so a unit test that compared it with the PSS
failed now and then. The benchmark now reads PSS and RSS from one `smaps_rollup` snapshot and only the peak from
`VmHWM`. One run on the same desktop and display (2026-10-26, load average ≈ 20 from other builds): PSS 275 MiB,
RSS 281 MiB, peak ≈ 365 MiB (first run: 280 / 288 / 369); the scroll phases ran at 119.8 / 117.9 / 116.5 fps with
UI work p95 ≤ 6.9 ms. In that run the window's first frame came 52 s after the process start (the view then showed
its first screen 1.6 s after it opened, under the load), and the first-screen figures above are those of the
first run. The M1 final review traced the 51 s before the view opened to the startup code before the window, most
likely the first-run format files: on a fresh configuration folder the GUI wrote them (`FormatStore::materialize`, D-21: 22 atomic
writes, each with a file and a folder flush) right after the benchmark had written about 100 MB into the same
encrypted btrfs volume under heavy load. Since then the GUI binds its IPC endpoint and opens the window first and
writes these files on a background thread after the first frame, so they no longer delay the window or the
endpoint (with a fresh configuration folder under Xvfb, at load 15–17, the first frame came after 0.28–0.61 s
before the change). The folder table's RSS column is from the
first run (`VmRSS`).

**Memory after views close.** Opening and closing views does not leak: a unit test
(`closing_views_releases_their_data`) checks that the comparison, the documents and the folder model are dropped
once a view closes, also while its job still runs. The process keeps memory the allocator has not returned to the
system: under Xvfb, 15 open/close cycles of a 1,000,000-line text comparison (closed after 0.3 s) and a
20,000-file folder comparison leave 350–420 MiB RSS with no view open (empty window: 72–87 MiB), and the figure does
not grow with more cycles (three rounds of five cycles: 349, 417, 388 MiB; with `MALLOC_ARENA_MAX=1`: 376, 377,
376 MiB; with glibc's mmap and trim thresholds at 64 KiB: 215, 271, 269 MiB). This is glibc's retention of freed
heap (the heap shrinks only from its top), not live data; returning it would need `malloc_trim` (an `unsafe`
FFI call or a crate that is not approved) or a different allocator, which is left to a later decision.

## 3. Crates explicitly rejected

| Crate | Reason |
|-------|--------|
| `content_inspector` | Unmaintained; replaced by in-house detector + `infer`. |
| `slint` | GPL-3.0 / commercial. |
| `unrar` (default) | Links non-OSI UnRAR source; opt-in feature only. |
| `pcre2` | C library with build complexity; `regex` + `fancy-regex` cover the needed subset. |
| `walkdir` alone | `ignore` provides a parallel walker on top of it; keep `walkdir` only transitively. |

## 4. Build and tooling

- `cargo fmt`, `cargo clippy -D warnings`, `cargo nextest run` (plus `cargo test
  --doc`) and `cargo deny` in CI on Linux, Windows, macOS (`.github/workflows/ci.yml`;
  `.config/nextest.toml` runs the wall-clock latency tests one at a time and retries
  them on CI, D-69). A `docs` job runs `cargo xtask commands-doc --check` and `cargo
  xtask help-check` (`xtask/`, D-68). The criterion benches and the G1/G2 gates run in
  `.github/workflows/bench.yml`, started by hand (25 % tolerance against a baseline);
  the reference figures in the docs come from runs on a fixed machine.
- Window tests and GUI measurements run headless with `env -u WAYLAND_DISPLAY
  xvfb-run -a …` (17-agent-workflow §3.5).
- Release builds with LTO, `codegen-units=1`, `panic=unwind` (we must survive
  panics in a job and report them).
- Packaging: AppImage/Flatpak (Linux), MSI/MSIX (Windows), DMG (macOS) — P1.
