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
| XML checks | `roxmltree` 0.21 (dev-only, `hikasa-report` tests), `xmllint` (libxml2, a CI tool, never linked) | MIT OR Apache-2.0 / MIT | Well-formedness and structure of the XML report dumps; `xmllint --schema` against `docs/schemas/hikasa-report-1.xsd` on Linux and macOS CI (M2 W7, D-199, D-211). |
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
| a signal-handling crate | M2: `ctrlc` (D-159), see the M2 table below. In M1 Ctrl-C terminated the CLI, and every `--output` file is written atomically. A cancelled comparison now ends with exit 100 before it renders, so `--output` keeps the report it had. |
| `tempfile` (dev-only, but for one call) | Atomic writes in library code are hand-rolled (temp file + fsync + rename). `hikasa-vfs` calls `TempPath::persist_noclobber` alone, in `local/mod.rs`, to give a new temporary file its name: a rename that never replaces has no safe spelling in std, and `unsafe` stays in `mmap.rs`. It holds `renameat2`, `renamex_np` and `MoveFileExW`, with a hard link where a file system has none (harmless for a temporary file of its own). Renaming a user's item onto a free name calls `rustix` directly instead, without the hard link (M2 table). |

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
| `zip` 9.2.0 (MIT) | W2, W5 | `default-features = false`, `deflate`, `deflate64`, `bzip2`, `zstd`, `lzma`, `xz` (W2: `deflate` only). No `aes-crypto`, `time`, `ppmd`, `rayon`, `legacy-zip`: encryption is reported, not implemented | settings packages (`.hikasa-pkg`, D-86) in `hikasa-session`, whose import still takes stored and deflate only; zip archives in `hikasa-vfs`, named only under `archive/` |
| `tar` 0.4.46 (MIT OR Apache-2.0) | W5 | `default-features = false` (no `xattr`) | `Header` and, for what a rebuild writes, `Builder`; reading is the in-house `archive/tar_walk.rs`, because `Entries` buffers extension headers without a limit |
| `flate2` 1.1.10 (MIT OR Apache-2.0) | W5 | `default-features = false`, `zlib-rs` | gzip streams; the pure-Rust back-end `zip` already enables, the features unify |
| `bzip2` 0.6.1 (MIT OR Apache-2.0) | W5 | default (`libbz2-rs-sys`, a Rust port; no C) | bzip2 streams |
| `lzma-rust2` **0.23.0** (Apache-2.0) | W5 | `default-features = false`, `std`, `encoder`, `optimization`, `xz` | xz streams. Not the newest 0.24.0: `zip` 9.2 needs `^0.23`, and one LZMA codec in the tree is the point. Moves when `zip` moves |
| `zstd` **0.14.3** (BSD-3-Clause) | W5 | `default-features = false` | zstd streams. Not the 0.13 of the plan: `zip` 9.2 needs `^0.14.2`. `zstd-safe` 8.2.0, `zstd-sys` 2.1.2 (BSD-3-Clause), libzstd 1.5.7 |
| `ctrlc` 3.5.2 (MIT OR Apache-2.0) | 0.1.91 | default (no `termination`: SIGTERM and SIGHUP end the process as before) | Ctrl-C of the CLI (D-159): `hikasa-cli` only, named only in `interrupt.rs`. New in `Cargo.lock`: `ctrlc`; `nix` and the Windows and Apple crates it needs were there |
| `open` 5.4.5 (MIT) | W6b | `default-features = false` | the associated application of FC-062 (D-186): `hikasa-session`, named only in `host.rs` (`SystemLauncher`, adapter-only) |
| `fontique` 0.11.1 (Apache-2.0 OR MIT) | 0.1.91 | `default-features = false`, `system`, `fontconfig-dlopen`, what eframe's system font fallback already compiles | the theme's named fonts (D-160): `hikasa-gui` only, named only in `fonts.rs`. No new package. On Linux it opens fontconfig at run time, so no build or start-up needs it; without it no family is found |
| `rustix` 1.1.5 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) | W8a review | `fs`, what `tempfile` already compiles (no new crate in `Cargo.lock`) | the kernel's no-replace rename of `hikasa-vfs` on Unix (`renameat2` `RENAME_NOREPLACE`, `renameatx_np` `RENAME_EXCL`): `rename(replace = false)` onto a free name; named only in `core/hikasa-vfs/src/local/unix.rs` (a plain dependency: the §0.5 table has no target sections) |

New crates in `Cargo.lock` with W5 besides these: `deflate64`, `libbz2-rs-sys`, `sha2`, `digest`, `block-buffer`,
`hybrid-array`, `typenum`, `const-oid`, `crypto-common`, `zstd-safe`, `zstd-sys`, and the build-time `cc`.

**C code.** `zstd-sys` compiles libzstd with the `cc` crate at build time: no cmake, no bindgen (pregenerated bindings),
no nasm; each build needs a C compiler for the target, nothing else. It is the only C in Hikasa's binaries, and no Hikasa
source is `unsafe` for it. `packaging/build-linux.sh` pins `CFLAGS_x86_64_unknown_linux_gnu`, which `RUSTFLAGS` does not
govern. Do not set `CC` to `zig cc` for Linux: the `cc` crate passes a target zig refuses.

**A licence exception, provisional.** `libbz2-rs-sys` is under `bzip2-1.0.6`, a permissive BSD-style licence `deny.toml`
did not list; no bzip2 back-end avoids it (the C original has the same licence). `deny.toml` holds a crate-scoped
exception for that crate alone, pending the maintainer's objection (D-154, as for `epaint_default_fonts`).

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

### M2 W4a: the shell's gates (D-132)

`env -u WAYLAND_DISPLAY xvfb-run -a cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture
--test-threads 1 text_scroll two_windows` (2026-10-30; the same desktop as above, the binary on Xvfb's default
640 × 480 screen with Mesa llvmpipe, the only renderer the agents may use; the GPU figures are the maintainer's run
at W14). The runs require `DISPLAY` and a software renderer. `two_windows` (`HIKASA_GUI_BENCH=two_windows`) opens a
second window with Home at the first screen (`WindowNew`), lets 240 frames pass and runs the three scroll phases in
the first window; every frame then also paints the second window (an immediate viewport).

| Gate | Run at load ≈ 9 (final) | Earlier runs | Threshold |
|---|---|---|---|
| `text_scroll` first screen | 0.16 s (coloured 0.45 s) | 0.31 s at load ≈ 300 | < 1 s: **met** |
| `text_scroll` memory | PSS 368 MiB (anon 315), RSS 417 MiB, peak 423 MiB | PSS 363 MiB | PSS < 381.5 MiB: **met** |
| `text_scroll` UI frame work | p95 0.44 / 1.0 / 1.13 ms, max ≤ 1.9 ms per phase | max 6.1 ms | p95 < 8, max < 16.7 ms: **met** |
| `two_windows` memory | PSS 375 MiB (anon 318), RSS 424 MiB, peak 429 MiB | 380 MiB; 404 MiB in a run that opened the second window after the coloured screen with a 30-frame warm-up, at load ≈ 300 | PSS < 381.5 MiB: **met** (narrowly) |
| `two_windows` UI frame work | p95 3.1 / 3.7 / 3.8 ms, max ≤ 6.9 ms per phase | max 14.9 ms; p95 20 ms in the 30-frame warm-up run (the second window's surface was being created during the first phase) | **met** |

Under llvmpipe the renderer's buffers live in the process: the single-window PSS is 363–368 MiB here against
275 MiB on the GPU (M1, above), and a second window adds 7–40 MiB of anonymous memory for its surface and its
copy of the font atlas, while its model (one Home tab) is a few kilobytes. The frame work of `two_windows`
includes the software rasterisation of the second window inside the root frame (immediate viewport), which a GPU
does asynchronously. The memory margin under llvmpipe is small (≈ 6 MiB); the maintainer's GPU run at the end of
W4c re-measures it.

**Re-run after the W4a review fixes (2026-10-30, load ≈ 250, same machine, llvmpipe over GL).** Both variants of
the gate were run: `two_windows` (second window at the first screen) and the new `two_windows_late`
(`HIKASA_PERF_SECOND_WINDOW=coloured`: second window once the coloured screen is up, the variant that measured
404 MiB before), with `text_scroll` as the single-window baseline in the same conditions. The bench was not
changed apart from that switch.

| Run | PSS (anon) | RSS / peak | UI frame work | Gate |
|---|---|---|---|---|
| `text_scroll` (one window) | 396.9 MiB (315.1) | 417.3 MiB | p95 ≤ 3.8 ms | PSS **not met** |
| `two_windows` (first screen) | 402.3 MiB (316.8) | 422.1 / 422.1 MiB | p95 4.68, max 9.99 ms | PSS **not met**; frames met |
| `two_windows_late` (coloured screen) | 401.4 MiB (315.5); again 401.5 (316.0) | 421.5 / 423.1 MiB | p95 4.7, max 8.9 (15.3) ms | PSS **not met**; frames met |

What the figures say (`/proc/<pid>/smaps` sampled during the runs): the anonymous memory is the same as in the
earlier passing runs (315–318 MiB) and the second window adds only ≈ 1–2 MiB of it at the report, plus ≈ 13 MiB of
llvmpipe colour buffers and a 3.7 MiB X shared-memory present image (1200 × 800 × 4) while it is shown — the
software renderer's per-surface buffers, which live in video memory on a GPU. The rise against the earlier runs
(PSS 368/375 MiB) is file-backed: the process's share of Mesa's `libLLVM` (39.8 MiB PSS), `libgallium` (12.0 MiB),
the probed Vulkan driver (3.9 MiB) and the binary (19.5 MiB) — about 82 MiB now against about 53 MiB then. PSS
divides a shared library's pages among the processes that map it, so the same program measures 25–30 MiB more
when fewer other processes on the machine map Mesa's libraries; the single-window `text_scroll` fails the limit in
this run too. No change in the GUI reduces those mappings (they are the software renderer), and the window size and
the scenario are the gate's; the gate is therefore **reported as failing under llvmpipe in this run**, the GPU
figure being the maintainer's run at the end of W4c (D-132). Forcing wgpu to GL only would drop the probed Vulkan
driver (≈ 4.6 MiB) but would change the renderer real users get, so it was not done.

### M2 W4b: gate G12 (D-132)

`workspace_restore` (release, Xvfb, llvmpipe, 2026-10-30, load ≈ 4–8 on the shared machine): 10 tabs in one window
through `hikasa-gui <workspace>` (5 sessions of 10,000-line pairs and 4 of 1,000-file trees behind the active tab).
First screen: active tab the 1M-line pair. Memory: two workspaces over the same files with a 1,000-line active pair,
one with the 9 pending tabs, one with the active tab alone, each sampled after the coloured screen with no job left
and 60 idle frames, three rounds, the median taken.

| Item | Gate | Measured |
|---|---|---|
| First screen of the active tab after the process start | < 1 s | 176 ms (268 ms in the previous run) |
| Views built / tabs pending | 1 / 9 (no job for the pending tabs) | 1 / 9 |
| (PSS_10 − PSS_1) / 9 | < 20 MiB | 0.05 MiB (rounds 0.10, 0.05, 0.00; PSS 138.2–138.5 vs 137.3–138.3 MiB) |
| The same in anonymous memory | reported | 0.03 MiB (rounds 0.14, 0.01, 0.03; 54.1–54.2 vs 52.9–54.0 MiB) |

Two whole runs gave the same median PSS. The run-to-run spread of one workspace is up to about 1 MiB, more than the
difference measured, so the honest reading is "under about 0.1 MiB per pending tab", not 0.05 exactly; a pending tab
holds one session value and starts no job. The earlier figure of this gate (first screen against `scroll`) sampled
during the 1M-line comparison, so it measured that job's progress and was withdrawn.

### M2 W4c: the Options gate and the wave-end run (D-132)

`env -u WAYLAND_DISPLAY xvfb-run -a cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture
--test-threads 1` (2026-10-31, the same desktop, Xvfb's default screen, Mesa llvmpipe 26.2 over GL, load 3–8 on the
shared machine, the six tests one after the other on one binary). `text_scroll`, `two_windows` and `two_windows_late` are
sampled in `memory_settled` (60 idle frames after the last phase) and give the median of three whole runs; every run:

| Gate | PSS per run (median) | anon | RSS | peak RSS | UI frame work, last run | Result |
|---|---|---|---|---|---|---|
| `text_scroll` | 255.4, 256.2, 253.6 (**255.4**) MiB | 190.1, 190.6, 187.9 | 293.7, 294.5, 291.8 | 423.4, 421.3, 419.2 | p95 0.41/0.99/1.15, max 0.61/1.13/1.33 ms; first screen 170 ms, coloured 409 ms | **met** |
| `two_windows` | 263.2, 258.5, 262.0 (**262.0**) MiB | 194.4, 191.8, 195.2 | 301.5, 296.8, 300.3 | 429.2, 423.4, 424.7 | p95 2.59, max 3.29 ms | **met** |
| `two_windows_late` | 259.0, 257.6, 255.0 (**257.6**) MiB | 190.2, 190.7, 188.4 | 297.3, 295.9, 293.2 | 423.8, 425.4, 423.2 | p95 2.60, max 4.30 ms | **met** |
| `options_memory` | 105.4 MiB (one run) | 41.4 | | | | < 120 MiB: **met** |
| `workspace_restore` (G12) | per pending tab 0.05, 0.02, −0.09 (median 0.02) MiB; anon median 0.07 | | | | 179 ms; 9 pending, 1 view built | **met** |
| `folder_scan` | PSS 130.7 MiB; scan 189 ms, UI work p95 1.84, max 1.84 ms, none above 16.7 ms | | | | | **met** |

**What the earlier "met" was worth.** The first W4c report gave one run each and called `two_windows` met at 359.6 MiB. It
was bimodal, at about 383 or 358 MiB. With the code of this review, before the change below, the same runs gave
`two_windows` 389.1, 360.2, 359.9 (median 360.2) and `two_windows_late` 385.9, 358.6, 382.5 (median **382.5: not met**).

**Cause.** The explanation of W4a (Mesa's file-backed share moving with what else maps it) was wrong for this swing. The
upper mode is one more anonymous region of about 22.6 MiB, peak RSS unchanged: glibc's retention. Its dynamic mmap
threshold rises to 32 MiB when the first large buffers are freed, and from then on buffers of that size come from the
arenas and stay in their free lists, whether a run returns one depends on what lies above it. With
`MALLOC_MMAP_THRESHOLD_=4194304` the region goes, the swing goes, and 125 MiB of the settled figure goes with it
(`two_windows_late` 261.7, 257.1, 258.1). D-134 sets it: the GUI replaces itself once at the start, because `mallopt` is
`unsafe` (architecture check 1) and another allocator is a dependency (D-133). The frame figures did not get worse.

**What this does not change.** The gate is a settled PSS. The **peak RSS is still 419–429 MiB, above 400 MB** in every
run: the comparison's own peak, which the threshold does not touch, and which the gate does not measure. On a platform
other than glibc there is no D-134 and these figures do not apply; the frame-rate item stays the maintainer's GPU run
(D-132). The exec happens before `process_start`, so its milliseconds are not in the first-screen figures.

### M2 W5a: gate G6 (D-155)

Release build, fixtures generated by the tests, warm cache, on 2026-10-31, measured again after the W5a review on
HEAD of that review. **The machine was quiet: load average 2.7 to 3.5** (the first measurement ran at 9 to 45, and its
figures are in the history of this file). Medians of three runs; the commands are in
`cli/hikasa-cli/tests/perf_gates.rs` and `core/hikasa-vfs/benches/archive_crc_1gb.rs`.

| Item | Measure | Threshold | Measured |
|---|---|---|---|
| G6 list | `hikasa vfs ls big.zip --json`, 100,000 stored files in one root | < 1 s | **0.310 s** (0.308–0.313) |
| G6 compare | `hikasa folder compare tree tree.zip --content crc --json`, 10,000 files of 4 KiB in 100 folders, equal | < 2 s | **0.033 s** (0.033–0.037; 10,000 content tests; the zip side decodes nothing) |
| G6 CRC | `prepare` of a 1 GiB `tar.gz` of one member against `MultiGzDecoder` to a sink, best of five each | ≤ 1.3 × | **1.014 ×** (1.148 s against 1.132 s; the fixture compresses 3.3:1) |
| G2 | the M1 gate, every file now meets `archive_format` | < 1.5 s | **0.051 s** (0.049–0.051), peak RSS 33 MiB |
| Memory | peak RSS of G6 list | ≤ 150 MiB | **125.4 MiB** |
| Memory | peak RSS of the index of 1 GiB through the binary (a stored stream, nothing spooled) | ≤ 64 MiB | **10.3 MiB** |
| Memory | an index, on any input | `entries_max` × 216 bytes + `paths_max`, 462 MiB at the defaults (§1.5.5) | the worst shape the default limits allow, 995,000 paths of 250 bytes: **696 MiB** of peak RSS while it is built, 1.45 GiB when all of it is listed. 512 bytes per member holds for paths under about 296 bytes |
| Cancel (G4) | `prepare`, a spool of a 1 GiB stream, a call waiting for an index or a decoder, a tree of 400,000 entries stop | within 100 ms | re-measured for 0.1.91, release build, load about 110, `archive_cache.rs` with `--nocapture`: `prepare` **0.05 ms**, spool **1.2 ms**, a waiter **0.12 ms**, a reader **4.3 ms**, the tree **11 ms** at worst. The earlier "under 1 ms" (load 16 to 40) does not hold at this load; the gate does |

G6 list writes a 39 MB document to a file on the test's disk. At load 250, with the disk busy, that write made the
median 6.2 s (0.4 s of CPU; the same run to `/dev/null` took 0.30 s), so on a machine that busy the gate reads the disk.
The first run was over the RSS budget (159 MiB): `write_json` held the whole document in memory, and now streams it
after one pass to a sink, so a value that cannot be serialised still writes nothing.

### M2 W6c: gate G7 (D-192)

Release build, fixtures generated by the tests, on 2026-11-02, on `feat/m2` after W6c's changes. The machine is shared: the
load average (1, 5, 15 min) is given with each round; medians of three inside each round. Commands:
`cargo nextest run --release -p hikasa-session --test perf_g7 --run-ignored only --no-capture`,
`cargo bench -p hikasa-folder --bench sync_plan_100k`, `cargo nextest run -p hikasa-folder --test ops_memory --no-capture`.

| Item | Measure | Threshold | Measured |
|---|---|---|---|
| G7 copy | `folder_op_execute` of a prepared plan copying 10,000 files of 4 KiB in 100 folders, local → local, targets absent, against `fs::copy` + `File::set_modified` in the same test, on `/var/tmp` | ≤ 1.5 × | **1.39–1.48 ×** in six rounds: 1.44, 1.48, 1.39 (load 1.2, quiet), 1.43, 1.45, 1.46 (load 6); 347–357 ms against 241–253 ms |
| G7 plan | `plan_sync` of a compared tree of 100,000 files on each side, half differing, Mirror to Right (`MemoryVfs`) | < 0.5 s | **0.188 s** median of five (0.168–0.202), criterion 0.169 s; load 5.4 |
| Memory | `SyncRow` | ≤ 16 bytes | **12 bytes** |
| Memory | `OpPlan` of 101,000 items (`folder op copy` of 100,000 files in 1,000 folders), live heap on a counting allocator | < 64 MiB | **56.2 MiB** (584 bytes an item); the `SyncPlan` of the same tree, rows and items, **57.5 MiB** |
| G2 | the M1 gate through the binary | < 1.5 s | **0.078 s** median (load 11); unchanged |

**G7 copy failed first, and was made to pass by doing less, not by checking less.** On W6b's path the same test gave
**1.52, 1.58, 1.52 ×** (load 5) and **1.54, 1.55, 1.53 ×** at load 1.8. `strace -y` of one file showed 15 calls against
the plain copy's 13, two of them repeats: `copy_new_file` stat'ed the source by its path again, a moment after the stamp's
`lstat` had seen it as a regular file, and `run_item` `lstat`ed the target to see it absent, which the rename that cannot
replace (W6a note 8) checks again at the commit, atomically. The source's size now travels in `HandleMeta::size`, and a
new file's absent target is checked by that rename, with the error of D-103 when it was there. What the item checks is
unchanged: its stamps, the folder above it, the fence, and a target that appeared since the plan stays. The other extra
calls are the guarantees: the folder above each item (D-94 rule 3), the stamp, a temporary name and its rename.

**After the review of W6c** (`LocalVfs::copy_new_file` reads through `take(size + 1)` and fails a count other than the
stamp's): the same test, two release binaries, the copy path before and after that change and nothing else different,
run in turn, six rounds each, load given before each. Load 14–19: before 1.49, 1.46, 1.46, 1.47, 1.41, 1.46 ×
(`folder_op_execute` 358–395 ms), after 1.48, 1.45, 1.48, 1.13, 1.48, 1.40 × (357–379 ms; the 1.13 is a slow plain copy).
At load 35–60 both sides crossed the gate in some rounds (before 1.43–1.58 ×, after 1.15–2.09 ×). No difference beyond
the noise of a shared machine; the gate keeps no margin, as before.

**Memory.** The first count gave 91.9 MiB for the `OpPlan`: the plan kept the capacity its vectors grew to (131,072 items
of 640 bytes), and a `Spec` was 112 bytes because of the URL variants no plan holds. The builder now gives back what it did
not use, and `Spec::Remote` and `Spec::Http` are boxed: a `Spec` is 56 bytes and a `PlanItem` 472.

### M2 W6d: the folder gate again (ARCH-GUI-003)

A6.34 asks the M1 folder gate again once the view runs operations. `env -u WAYLAND_DISPLAY xvfb-run -a cargo test
--release -p hikasa-gui --test perf_gate -- --ignored --nocapture --test-threads 1 folder_scan` (2026-11-02, the same
desktop, Mesa llvmpipe over GL, shared machine), three runs of W6d and, for comparison, two of the commit before it (the
end of W6c), alternating as the load fell:

| Build | Load | Scan | UI work p95 / max | PSS | GUI RSS |
|---|---|---|---|---|---|
| W6d | 3–11 | 481 ms | 3.38 / 3.38 ms | 164.9 MiB | 170.1 MiB |
| W6d | 2.8 | 194 ms | 2.72 / 2.72 ms | 149.2 MiB | 169.6 MiB |
| W6d | 2.6 | 189 ms | 2.18 / 2.18 ms | 149.4 MiB | 169.8 MiB |
| W6c end | 2.7 | 494 ms | 3.08 / 3.08 ms | 165.3 MiB | 169.7 MiB |
| W6c end | 3.2 | 195 ms | 2.45 / 2.45 ms | 149.8 MiB | 168.9 MiB |

**Met**, and unchanged by W6d: no frame above 16.7 ms, the same figures as the build before it. Both builds show the same
two modes (about 149 and 165 MiB; here the first run of each build was the upper one). The PSS is above W4c's 130.7 MiB with
the parts in between (W5, W6a–c), not with this one.

### 0.1.91: named fonts and the Options gate (D-160)

Release builds under Xvfb with llvmpipe, one window with Home, `HIKASA_GUI_BENCH=options`, load 110 to 280 from other
projects on the machine. Three runs each, in turn; PSS and anonymous memory in MiB.

| Binary | PSS | anon | Gate < 120 MiB |
|---|---|---|---|
| 0.1.91, no named font | 122.6, 122.8, 121.5 | 40.5, 40.5, 40.6 | **not met** |
| 0.1.91, editor font `DejaVu Sans Mono` | 122.6, 124.7, 121.5 | 40.1, 42.1, 40.1 | **not met** |
| 7467fbb, the commit before both features | 120.2, 123.0, 122.8 | 38.5, 40.5, 40.5 | **not met** |
| the 0.1.90 package (stripped) | 121.8, 120.3, 110.0 | 40.5, 38.5, 38.5 | not met in two of three |

`perf_gate options_memory` itself gave 137.0 MiB (anon 40.5) on an unstripped build at load 215, where one run of the
named-font test gave 133.5 (anon 42.0). So **the gate does not hold on this machine today, and it did not before these
features**: the W4c figure of 105.4 MiB (anon 41.4) does not reproduce. The anonymous part has not moved; the excess is
the file-backed part, 80 to 96 MiB against 64 then. It swung by 10 MiB between runs of one binary within minutes, as a
proportional share does when other processes map the same libraries or leave them, and stripping took off about 2 MiB.

**The cause is the proportional share of the llvmpipe libraries.** PSS divides a file-backed page by the processes that map
it, and the software renderer's libraries (`libgallium`, LLVM) are most of what this process maps from files. On this
machine 21 other processes mapped `libgallium`, a number that follows what else runs. For one binary, one configuration:
RSS 140 to 143 MiB, anonymous 38.5 to 40.6, PSS 108 to 120, and about 137 once no other process shared those libraries.
**So the gate as defined is not a stable measure on a machine shared with other GUI processes**: the same memory counts
between 108 and 137 MiB by who else is running, and 105.4 MiB was a good moment, not a better build. The limit stays as it
is, and the maintainer's run on a GPU, where no renderer library of that size is mapped, decides it.

`memory_settled` also reports a figure that does not move with the neighbours, `own_mib`: anonymous memory plus the resident
pages of the binary itself, counted whole, without any library, next to `total_pss_mib`. It tells a change of this program
from a change of its surroundings; it is a measure beside the gate, not a gate.

A named font adds no measurable PSS and at most 1.6 MiB of anonymous memory, in the size of the runs' own noise: the
enumeration is dropped after the lookup and the font file stays mapped, here 343 KiB. A lookup on this machine (1,247
font files through fontconfig) takes **221 ms** the first time in a process and **18 ms** after that, on its own thread;
the window shows the built-in font until it answers. The `ctx.set_fonts` of an apply runs on the UI thread and was not
timed; it happens once per change of family, never in a frame of its own.

### M2 W9a: gate G15 (tree, select, completion, browse; D-257)

`env -u WAYLAND_DISPLAY xvfb-run -a cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture
--test-threads 1 <name>` (2026-11-05, the same desktop, Mesa llvmpipe, **loaded machine**: other projects' builds and
tests ran meanwhile, load 4.7 to 12.7; one run each, no repeats to fight the noise). New modes
`HIKASA_GUI_BENCH=folder_columns|select|completion|browse` (§1.9.25 note 20).

| Row | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G15 tree (`folder_columns`) | 5.9 | scan 538 ms, 8 frames while scanning: UI work p95 29.95 / max 29.95 ms (2 above 16.7 ms), tree rebuild max 33.1 ms; PSS 140.1 MiB | max ≤ 16.7, p95 < 8 ms; PSS < 381.5 MiB and ≤ M1 + 48 MiB | **not met** (frames); PSS met |
| same tree, tree order, default columns (`folder_scan`, for comparison) | 5.6 | scan 439 ms, 18 frames: p95 7.03 / max 8.35 ms, rebuild max 8.1 ms; PSS 135.7 MiB | (M1 (b)) | met |
| G15 select | 5.7 | Select All Files 65.4 ms, Invert Selection 112.0 ms, Shift+Left 41.8 ms (one frame each, 111,150 rows); 300 × Shift+Down p95 1.37 / max 2.06 ms | each command ≤ 16.7 ms; Shift+Down p95 < 4 ms | **not met** (commands); Shift+Down met |
| G15 completion | 4.7 | the list 171 ms after the last key (the 150 ms debounce included); typing max 1.38 ms | ≤ 200 ms; no frame above 16.7 ms | **met** |
| G15 browse | 5.3 | listed 288 ms after it opened (100,000 entries); 600 frames of scrolling p95 2.70 / max 5.14 ms; PSS 179.3 MiB | ≤ 1 s; max < 16.7 ms | **met** |
| ARCH-GUI-003 `text_scroll` | 7.4–9.5 | PSS 257.7, 262.1, 262.7 (**262.1**) MiB; 221.9 / 172.7 / 122.8 fps; first screen 281 ms, coloured 895 ms | < 381.5 MiB, ≥ 60 fps, < 1 s | met |
| ARCH-GUI-003 `two_windows` | 11.3–12.5 | PSS 264.4, 261.9, 267.6 (**264.4**) MiB; last run UI work p95 6.9, max 17.38 ms | < 381.5 MiB, p95 < 8, max < 16.7 ms | PSS met; **max not met** (one frame, at load 12) |
| G12 `workspace_restore` | 12.5 | per pending tab 0.03 MiB PSS, 0.00 anonymous; first screen 420 ms | ≤ 20 MiB a tab | met |

**What fails and why.** The tree row fails in the sort, not in the columns or the drawing: with Size descending every
rebuild while scanning sorts 111,150 rows (`visible_rows_sorted`, max 33 ms), where the tree order's rebuild takes 8 ms
on the same tree at the same load. The select row fails in the selection's whole-tree work on the UI thread: each of the
three commands builds a `Selection` over every shown row, `retain_shown` copies it through a hash set of the 111,150
shown rows, and the status counts walk it once; Shift+Down, which touches one row, is far inside its budget. Both are
reported as found (docs/17 §7.1); the thresholds stay. The remedy — a rebuild that re-sorts only the folders whose
children changed, and a selection stored as ranges of the shown rows (or the whole-tree commands in a job) — is left to
the maintainer's decision for W9b/W9c. `two_windows`' one 17.4 ms frame came at load 12 with the other figures as in W4c;
it is not re-measured on the real display by an agent (D-132). The PSS figures are W4c's (255–262 MiB) within the noise.

### M2 W9a after the review: gate G15 again (D-257)

The same command and desktop (2026-11-05, Mesa llvmpipe; other sessions' builds and tests ran meanwhile, load 4.1 to
8.8; one run each), after the review's fixes (§1.9.25 notes 23–35): each folder's sorted order cached between rebuilds,
the rows rebuilt off the window's thread while the comparison runs, the frame's command context computed once, the
selection's flags memoised, a dense `Selection` and raw select passes, the counts' totals, and Select All measured as
§1.9.21 says (Select All Files kept as an extra figure).

| Row | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G15 tree (`folder_columns`) | 4.3 | scan 963 ms, 15 frames while scanning: UI work p95 9.54 / max 9.70 ms (none above 16.7 ms); PSS 169.1 MiB | max ≤ 16.7, p95 < 8 ms; PSS < 381.5 MiB and ≤ M1 + 48 MiB | max **met**; p95 **not met**; PSS met |
| same tree, tree order, default columns (`folder_scan`) | 4.1 | scan 452 ms, 15 frames: p95 6.19 / max 6.44 ms, rebuilds max 5.1 ms; PSS 152.8 MiB | (M1 (b)) | met |
| G15 select | 4.3 | Select All 10.7 ms, Invert Selection 3.1 ms, Shift+Left 0.8 ms (one frame each, 111,150 rows); 300 × Shift+Down p95 1.04 / max 1.36 ms; Select All Files (extra) 7.0 ms | each command ≤ 16.7 ms; Shift+Down p95 < 4 ms | **met** |
| G15 completion | 4.2 | the list 165 ms after the last key (the 150 ms debounce included); typing max 1.26 ms | ≤ 200 ms; no frame above 16.7 ms | **met** |
| G15 browse | 4.1 | listed 267 ms after it opened (100,000 entries); 600 frames of scrolling p95 2.17 / max 2.80 ms; PSS 193.8 MiB | ≤ 1 s; max < 16.7 ms | **met** |
| ARCH-GUI-003 `text_scroll` | 4.5–4.9 | PSS 271.9, 271.6, 272.4 (**271.9**) MiB; 215.7 / 172.2 / 156.1 fps; first screen 261 ms, coloured 937 ms | < 381.5 MiB, ≥ 60 fps, < 1 s | met |
| ARCH-GUI-003 `two_windows` | 7.2–8.8 | PSS 282.7, 278.3, 276.0 (**278.3**) MiB; last run UI work p95 5.52, max 7.59 ms | < 381.5 MiB, p95 < 8, max < 16.7 ms | met |
| G12 `workspace_restore` | 8.8 | per pending tab −0.02 MiB PSS, −0.02 anonymous | ≤ 20 MiB a tab | met |

**What still fails and why.** The tree row's frames are no longer the sort's: the rebuild runs on its own thread while
the comparison runs (max 8 ms there, off the window's thread) and sorts again only the folders that changed; the worst
frames now are the drawing of a window whose sorted rows change under the pointer while the scan adds sizes (new texts
to lay out each rebuild, every column shown, llvmpipe). The tree order with the default columns stays at 6.2 ms on the
same tree at the same load; the step from 6.2 to 9.5 ms is that drawing. Reported as found (docs/17 §7.1), threshold
unchanged; not re-measured on a real GPU by an agent (D-132). The scan itself took longer with every column and the sort
(963 ms against 452 ms in the tree order): not a gate figure. The PSS figures of `text_scroll` and `two_windows` are
within W4c's and W9a's noise plus the W9a code (+10 to +14 MiB against the first W9a run).

### M2 W9b: gate G15 (preview, sync; D-257)

The same command and desktop (2026-11-05, release build, `env -u WAYLAND_DISPLAY xvfb-run -a`, Mesa llvmpipe; other
sessions' builds ran meanwhile, load 3.2 to 7.1; one run each). New bench modes `preview` (the `folder_scan` tree,
every row shown, Select All on the left, Copy to Right: ≈100,000 file items, every one replacing) and `sync_view` (a
Folder Sync view of the same tree, `--sync`, Mirror to Right; then Leave Alone on one row). No new crate (D-258).

| Row | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G15 preview | 4.4 | 100,021 lines (100,020 items and the summary); the preview's first frame 1,281 ms after the command (the plan job included); 600 frames of scrolling p95 1.35 / max 2.21 ms; PSS 165.4 MiB before, 234.8 MiB while open (+69.3 MiB); the preview's own memory 2 bytes an item (`folder_preview.rs`, counting allocator) | first frame < 1.0 s; max < 16.7 ms; ≤ view + 96 MiB; ≤ 16 B an item | first frame **not met**; scrolling, PSS and memory **met** |
| G15 sync | 4.7 | plan shown 145 ms after the comparison ended; an override of one row re-planned and repainted in 234 ms; 600 frames of scrolling p95 0.67 / max 1.01 ms; PSS 154.0 MiB | ≤ 0.5 s; ≤ 0.5 s; max < 16.7 ms | **met** |
| G15 tree (`folder_columns`), again | 5.1 | scan 474 ms, 13 frames while scanning: UI work p95 9.28 / max 14.28 ms; PSS 157.5 MiB | max ≤ 16.7, p95 < 8 ms | max met; p95 **not met** (as after W9a) |
| G15 select, again | 5.8 | Select All 12.3 ms, Invert 4.2 ms, Shift+Left 0.9 ms; Shift+Down p95 1.54 / max 1.89 ms; Select All Files 8.3 ms | each ≤ 16.7 ms; p95 < 4 ms | met |
| G15 completion, again | 7.1 | the list 165 ms after the last key; typing max 1.78 ms | ≤ 200 ms; ≤ 16.7 ms | met |
| G15 browse, again | 6.7 | listed 287 ms; scrolling p95 2.32 / max 3.29 ms; PSS 191.4 MiB | ≤ 1 s; < 16.7 ms | met |

**What fails and why.** The preview's first frame waits for the plan job, and the plan is core's `plan_op` of W6
(`folder_op_plan`): a release run of the Service alone on a tree of 100,000 files in 1,110 folders gave 0.62–0.64 s for
the plan, 2 ms for `folder_op_preview` and 1 ms for `folder_confirmation`; the bench's tree has ten times the folders and
the machine was loaded, hence 1.28 s. The window adds one frame (the dialog's first frame drew in 3.4 ms). Bringing the
row under 1 s needs a faster planner (W6's item builder: target stats and specs per item), not a change of the dialog;
reported as found (docs/17 §7.1), threshold unchanged. The tree row's p95 is W9a's open finding (§1.9.25 W9a note 34);
no profiler (`perf`) is installed on this machine, so it was not profiled further.

**After the W9b review** (the same command and desktop, 2026-11-05, on `feat/m2` after the review's fixes; other
sessions' builds ran meanwhile, load 6.7 to 14.1; one run each). The planner no longer asks the disk per replacing item
(§1.9.25 W9b note 30): core's `folder_op_plan` of the bench's tree went from 0.77–0.86 s to 0.30–0.32 s
(`core/hikasa-session/tests/perf_plan.rs`, ignored, release, the reviewer's G15-shaped tree of 100,020 files a side;
load 6.5 to 29), the plan unchanged. The bench now reports the plan job's own time apart from the first frame.

| Row | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G15 preview | 14.1 | 100,021 lines; the preview's first frame **330.7 ms** after the command, of it the plan job 324.8 ms; 600 frames of scrolling p95 1.67 / max 3.04 ms; PSS 124.1 MiB before, 208.4 MiB while open (+84.3 MiB) | first frame < 1.0 s; max < 16.7 ms; ≤ view + 96 MiB | **met** |
| G15 sync | 6.9 | plan shown 200.8 ms after the comparison ended; an override re-planned and repainted in 274.0 ms; PSS 134.6 MiB | ≤ 0.5 s; ≤ 0.5 s; max < 16.7 ms | **met** |
| G15 tree (`folder_columns`) | 6.7 | scan 384 ms, 10 frames while scanning: UI work p50 2.04 / p95 = max 14.43 ms (one rebuild frame; 10 frames make the p95 the maximum); PSS 149.9 MiB | max ≤ 16.7, p95 < 8 ms | max met; p95 **not met** (W9a's open finding, out of the review's scope) |
| G15 select | 9.0 | Select All 14.9 ms, Invert 4.6 ms, Shift+Left 1.3 ms; Shift+Down p95 1.97 / max 3.49 ms; Select All Files 7.4 ms | each ≤ 16.7 ms; p95 < 4 ms | met |
| G15 completion | 11.6 | the list 165 ms after the last key; typing max 1.38 ms | ≤ 200 ms; ≤ 16.7 ms | met |
| G15 browse | 12.1 | listed 320 ms; scrolling p95 4.02 / max 6.96 ms; PSS 170.4 MiB | ≤ 1 s; < 16.7 ms | met |

A node grew from 160 to 176 bytes (each side's identity fingerprint, 8 bytes; M1's budget is 256 bytes a node); the PSS
figures above are within their bounds, and the preview's +84.3 MiB within its 96.

**G1, G2, G4 and G7 after the W9b review** (release builds on `feat/m2` after the review's fixes, the commands of their
own sections; one round each, the load average before and after each round given, other sessions' builds running
meanwhile unless the load says otherwise). The planner's change touched the model's file identity, so the gates that run
through the model and the copy path were taken again.

| Item | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G1 through the binary (`perf_gates.rs`, 2026-11-05) | 11.7 → 5.5 | auto **0.982 s** median (0.972–0.999), standard 0.989 s, identity 0.826 s; peak RSS 300 MiB (bound 427 MiB) | < 3 s; RSS bound | met |
| G1 bench (`text_compare`, 2026-11-05) | 28.8 → 26.4 | best of three: auto 2.019 s, standard 1.940 s, identity 1.142 s | < 3 s | met |
| G2 through the binary (2026-11-05) | 11.7 → 5.5 | warm **0.083 s** median (0.083–0.094), `--json` 0.100 s; peak RSS 37 MiB | < 1.5 s warm | met |
| G4 folder (`cancel_latency`, 2026-11-05) | 24.7 → 28.8 | worst latency: `folder_100k` 21.1 ms, `huge_folder_1m` 86.7 ms over 15 cancellations, `restat_200k` 2.3 ms | ≤ 100 ms | met |
| G7 plan (`sync_plan_100k`, 2026-11-05) | 26.6 → 24.7 | median of five **0.227 s** (0.204–0.256), criterion 0.231 s | < 0.5 s | met |
| G7 copy (`perf_g7.rs`, 2026-11-05) | 5.5 → 26.6 | 1.61 ×: `folder_op_execute` 893.7 ms against the plain copy's 555.2 ms (both twice their quiet figures; the load rose from 5.5 to 26.6 during the round) | ≤ 1.5 × | not met, under load |
| G7 copy again (2026-11-08) | 3.8 | **1.41 ×** and **1.43 ×** in two rounds: `folder_op_execute` 357.8 and 350.2 ms against 253.1 and 244.8 ms | ≤ 1.5 × | **met** |

The G7 copy round of 2026-11-05 ran while the load rose fivefold, and the plain copy itself took twice its quiet time.
Nothing on the copy's path changed in the review: the planner's identity check applies to items that replace a file
(G7's targets are absent), the plan is made before the clock starts, and no commit of the review touched execution. At
load 3.8 the gate gives W6c's figures (1.39–1.48 ×, 347–357 ms against 241–253 ms); as W6c found, it keeps no margin
on a loaded machine. The G15 tree row (`folder_columns`) taken again at load 3.8 (2026-11-08): scan 340 ms, 22 frames while scanning, UI work
p50 0.87 / p95 **4.69** / max **4.97 ms**, the tree's rebuild max 6.0 ms, PSS 159.5 MiB: **met** in both its frame
figures, where the round above at load 6.7 had one rebuild frame of 14.43 ms among 10. Every earlier round missed the
p95 (W9a's open finding, at load 4.3–6.7); one round that meets it does not close the finding.

### M2 W9c: gate G15 and ARCH-GUI-003 at the end of the wave (D-257)

The whole G15 table and ARCH-GUI-003 once more at the end of W9c (§1.9.21, A9.27): release build of `feat/m2` after
W9c's commits, `env -u WAYLAND_DISPLAY xvfb-run -a cargo test --release -p hikasa-gui --test perf_gate -- --ignored
--nocapture --test-threads 1 --exact <row>`, llvmpipe only (D-132), one run each (`text_scroll`, `two_windows`: three
runs, the median), 2026-11-08; each row started once the 1-minute load average was below 6 or after waiting for it
(the load given is the row's own; `two_windows` started at 5.96, other sessions' builds pushed it to 8.6 meanwhile). The trees and inputs are the rows' own (the scan's 100,020 files a side in 11,110
folders; the preview's and the sync view's trees of W9b; 10,000 and 100,000 entries for completion and browse).

| Row | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G15 tree (`folder_columns`) | 6.1 | scan 245 ms, 7 frames while scanning: UI work p95 6.88 / max 6.88 ms (none above 16.7 ms); PSS 161.1 MiB (M1's `folder_scan` 130.7 MiB + 48 = 178.7) | max ≤ 16.7, p95 < 8 ms; PSS < 381.5 MiB and ≤ M1 + 48 MiB | **met** |
| G15 select | 6.1 | Select All 3.8 ms, Invert Selection 0.8 ms, Shift+Left 0.4 ms (one frame each); 300 × Shift+Down p95 0.42 / max 0.49 ms; Select All Files (extra) 3.4 ms | each command ≤ 16.7 ms; Shift+Down p95 < 4 ms | **met** |
| G15 preview | 6.5 | 100,021 lines; the preview's first frame 153.9 ms after Copy to Right (the plan job 153.6 ms of it); 600 frames of scrolling p95 0.40 / max 0.73 ms; PSS 149.3 MiB before, 227.7 MiB while open (+78.4 MiB) | first frame < 1.0 s; max < 16.7 ms; ≤ view + 96 MiB | **met** |
| G15 sync | 5.1 | plan shown 58.7 ms after the comparison ended; an override re-planned and repainted in 85.9 ms; 600 frames of scrolling p95 0.21 / max 0.35 ms; PSS 163.6 MiB | ≤ 0.5 s; ≤ 0.5 s; max < 16.7 ms | **met** |
| G15 completion | 5.8 | the list 157.0 ms after the last key (the 150 ms debounce included); typing max 0.37 ms | ≤ 200 ms; no frame above 16.7 ms | **met** |
| G15 browse | 5.7 | listed 201.7 ms after it opened (100,000 entries); 600 frames of scrolling p95 0.82 / max 1.16 ms; PSS 195.5 MiB | ≤ 1 s; max < 16.7 ms | **met** |
| ARCH-GUI-003 `text_scroll` | 6.0–7.3 | PSS 273.6, 277.2, 280.0 (**277.2**) MiB; 405.8 / 329.0 / 302.7 fps; first screen 265 ms, coloured 546 ms | < 381.5 MiB, ≥ 60 fps, < 1 s | **met** |
| ARCH-GUI-003 `two_windows` | 7.6–8.6 | PSS 290.4, 284.9, 282.2 (**284.9**) MiB; last run UI work p95 4.02, max 13.21 ms | < 381.5 MiB, p95 < 8, max < 16.7 ms | **met** |
| G12 `workspace_restore` | 5.0 | per pending tab −0.02 MiB PSS, −0.01 MiB anonymous (three rounds); first screen 199 ms | ≤ 20 MiB a tab | **met** |

W9c adds no work to a frame while no menu is open and nothing is dragged: the pane menus are built only while one is
open, the drop targets are a lookup of the row under the pointer, and the command context gains the file-manager flags
of the current row (two lookups). No crate was added (D-258): `Cargo.lock` is unchanged and this document gains no
dependency line.

G1, G2, G4 and G7 were not measured again: W9c changes none of their paths (core gains the clipboard's file lists, the
launcher's reveal, `check_folder_rules` and tests; the comparison, the scan, the plan and the copy are untouched since
their figures *after the W9b review* above).

### M2 W9c after the review: the rows the fixes touch (D-257)

The fixes add to every frame the window's check of how fresh the pointer is for a drag from another program (one scan of
the frame's events), the paste key's state and, in the folder tree, the keyboard's context-menu keys; the command
context gains two side fields. Measured as above (release, `env -u WAYLAND_DISPLAY xvfb-run -a`, llvmpipe, 2026-11-08,
the machine loaded by other sessions: 1-minute load 5.0–7.2 falling from 18 at the first run):

| Item | Load | Measured | Threshold | Status |
|---|---|---|---|---|
| G15 tree (`folder_columns`), first run | 7.2 | scan 476 ms, 2 frames while scanning: UI work p95 = max 8.0 ms; PSS 169.4 MiB | max ≤ 16.7, p95 < 8 ms; PSS < 381.5 MiB | max met, p95 at the limit (two frames) |
| G15 tree (`folder_columns`), again | 5.0–6.5 | scan 265 ms, 11 frames while scanning: UI work p95 7.37 / max 7.37 ms; PSS 153.6 MiB | same | **met** |
| G15 select | 5.0 | Select All 4.1 ms, Invert Selection 0.9 ms, Shift+Left 0.3 ms; 300 × Shift+Down p95 0.54 / max 1.68 ms; Select All Files 4.0 ms | each ≤ 16.7 ms; p95 < 4 ms | **met** |
| ARCH-GUI-003 `two_windows`, first run | 5.0–5.9 | PSS 274.5, 274.0, 271.4 (**274.0**) MiB; last run UI work p95 4.24, max 23.15 ms | < 381.5 MiB, p95 < 8, max < 16.7 ms | max **not met** (one frame) |
| ARCH-GUI-003 `two_windows`, again | 5.2–9.1 | PSS 256.4, 259.0, 279.9 (**259.0**) MiB; last run UI work p95 2.86, max 8.26 ms | same | **met** |

The first runs started while the 15-minute load was still 15 from other sessions; the second ones, minutes later, met
every row with the code unchanged, so the single slow frame is read as the shared machine's (the GPU run stays the
maintainer's, §6.9). No crate was added: `Cargo.lock` is unchanged.

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
- Packaging: local scripts in `packaging/` (`packaging/README.md`), no CI job and no new crate (D-133): `.tar.gz`,
  `.deb`, AppImage and an Arch `PKGBUILD` on Linux, a portable zip and an installer on Windows (x64 and x86). Tools:
  `zig cc` as the linker for an old glibc floor; mingw-w64 with `windres` for the Windows targets and their resources;
  `wixl` from msitools for the MSI installers; pinned downloads of appimagetool and the AppImage runtime. Flatpak, MSIX
  and DMG stay P1.

### M2 W10a: gate G8 (edit rows) and G1, G4, G5 on the new structures (D-271)

Release build of `feat/m2` at the end of W10a, inputs generated by the tests (the G1 pair `c_pair(1,000,000,
0x5EED_0001)`, c-cpp, Auto → Standard), `cargo test --release -p hikasa-text --test gates -- --ignored --nocapture
--test-threads=1` and `cargo test --release -p hikasa-cli --test perf_gates g1_text -- --ignored --nocapture`,
2026-11-08, one round each, other sessions' builds running (1-minute load given per row). W10a adds no crate:
`Cargo.lock` is unchanged and this document gains no dependency line (D-291).

| Item | Load | Measured | Threshold | Result |
|---|---|---|---|---|
| G1 through the binary (`perf_gates.rs`) | 3.6 → 6.0 | auto **0.470 s** median (0.452–0.472), standard 0.472 s, identity 0.377 s; peak RSS 301 / 301 / 279 MiB (bound 427) | < 3 s; RSS bound | met (block line table and chunked rows) |
| G1 in-process (`g1_text_compare_1m`, 8 threads) | 3.8 | best of three: auto 0.339 s, standard 0.331 s, identity 0.226 s | < 3 s | met |
| G4 text (`g4_cancel_latency`, `…_adversarial`) | 3.8 | worst 10.98 / 11.18 / 15.92 ms (auto, standard, identity); adversarial worst 79.17 ms | ≤ 100 ms | met |
| G5 (`g5_gui_enablers`) | 3.9 | `row` 8.2 ns on the chunked store, `FilteredView::get` 17.9 ns, `inline` p99 6.7 µs | < 200 ns, < 200 ns, < 50 µs | met |
| G8 type (one character into a same line, 20 places) | 3.9 | recompare median **36.2 ms**, worst 41.7 ms; `note_edit` median 0.052, worst 0.721 ms | every one < 50 ms; `note_edit` < 1 ms | **met** |
| G8 changed (one character into a changed line, 20 places) | 3.9 | recompare median **24.6 ms**, worst 28.3 ms; `note_edit` worst 0.079 ms | < 50 ms each | **met** |
| G8 lines: Enter in a line | 4.4 | recompare median 34.1 ms, worst **1,325 ms** (one of 20: an edit in the common prefix of a large nested node, which re-runs); `note_edit` worst 0.903 ms | < 50 ms each | **not met** |
| G8 lines: delete a line | 4.2 | median 35.1 ms, worst **1,123 ms** (as above) | < 50 ms each | **not met** |
| G8 lines: paste 100 lines | 3.8 | median **1,214 ms**, worst 1,639 ms: each paste shifts 100 lines, which wears the window band's 256-line margin after two pastes and the root re-runs | < 50 ms each | **not met** |
| G8 copy (Copy Section to Right, ≤ 10 rows, 20 sections) | 4.4 | median 44.2 ms, worst **1,087 ms** | < 50 ms | **not met** |
| G8 undo (1,000 edits with a recompare after every 50, 1,000 undos with `note_edit`, one recompare) | 3.9 | 1.483 s, the result equal to the original; the final recompare re-runs the root (the undos touch its common prefix), ≈ 1.3 s | < 1 s; equal | equal met, time **not met** |
| G8 memory | 3.9 | peak RSS growth over the 1,000 edits and recompares 0 MiB; live heap growth by a counting allocator on 100,000 lines (`edit_memory.rs`, CI) 2.24 MiB per 1,000 edits; line table 5.07 bytes a line, +8 a line in edited blocks | < 10 MiB; < 10 MiB a 1,000 edits | met |
| G8 worst (a character in the common prefix, reported) | 3.9 | 1,305 ms (the root re-runs: G1's align phase) | reported; ≤ G1's align phase | reported |

What misses: every recompare that the certificates of §1.10.11 keep incremental takes 25–45 ms at 1,000,000 lines
(the recorded derivation of the G1 pair nests four large nodes; each certified level costs a few ms of fingerprint and
window checks, and the rows are re-derived in about 13 ms). The misses are the edits whose certificate fails and whose
failing node is large: an edit inside a large node's common prefix or suffix (condition 1), and shifts that wear the
window band's margin (condition 3). Those run the engine on the whole node (≈ 1.1–1.6 s), which is exact by
definition. A wider band (1,024 lines) was tried: 587,178 band pairs at the root slowed every recompare to about 49 ms
and did not save the 100-line pastes. Open: certify a prefix shrink as candidate additions, keep the band's margin by
re-deriving it from the node's keys, and splice the rows instead of re-deriving them (W10b/W10c or a follow-up).

### M2 W10a review fix: gate G8 re-measured (and G1, G4, G5)

The W10a code after its review fix (contract §1.10.29 W10a notes 22–39: edge-delta descent, an exact window per
diagonal run, interned key ids, a shared derivation, the Step 3 splice, the work guard, F10), release build,
`cargo nextest run --release -p hikasa-text --test gates --run-ignored only --test-threads 1` and
`cargo test --release -p hikasa-cli --test perf_gates g1_text -- --ignored`, 2026-11-09. Other projects' builds ran
the whole time: the 1-minute load is given per row (6 to 72); one round each.

| Item | Load | Before the fix | After the fix | Threshold | Result |
|---|---|---|---|---|---|
| G1 through the binary | 11 | auto 0.470 s | auto **0.537 s** median, standard 0.519 s, identity 0.436 s; peak RSS 301 / 301 / 280 MiB | < 3 s; RSS bound 427 MiB | met |
| G1 in-process (8 threads) | 72 | auto 0.339 s | best of three: auto 0.395 s, standard 0.393 s, identity 0.225 s | < 3 s | met |
| G4 text | 70 | worst 15.9 ms; adversarial 79 ms | worst 12.7 ms; adversarial 95.3 ms | ≤ 100 ms | met |
| G5 | 70 | `row` 8.2 ns, `get` 17.9 ns, `inline` p99 6.7 µs | `row` 6.5 ns, `get` 18.1 ns, `inline` p99 6.1 µs | < 200 ns, < 200 ns, < 50 µs | met |
| G8 type | 39 | median 36.2, worst 41.7 ms | median **8.6 ms**, worst 9.8 ms; `note_edit` worst 0.81 ms | < 50 ms each; < 1 ms | **met** |
| G8 changed | 39 | median 24.6, worst 28.3 ms | median **7.2 ms**, worst 14.1 ms; `note_edit` worst 0.09 ms | < 50 ms each | **met** |
| G8 lines: Enter | 66 | median 34.1, worst 1,325 ms | median **6.7 ms**, worst 1,259 ms (one of 20 takes the full compare); `note_edit` worst 1.30 ms | < 50 ms each | **not met** |
| G8 lines: delete a line | 61 | median 35.1, worst 1,123 ms | median **7.1 ms**, worst 1,434 ms (one of 20, as above); in a quieter run (load 8) worst 48.1 ms | < 50 ms each | **not met** in this run |
| G8 lines: paste 100 lines | 41 | median 1,214 ms | median 1,563 ms, worst 2,481 ms: every paste takes the full compare (work guard); `note_edit` median 1.31 ms | < 50 ms each | **not met** |
| G8 copy (≤ 10 rows) | 72 | median 44.2, worst 1,087 ms | median **16.6 ms**, worst 1,352 ms (full compare); `note_edit` worst 1.26 ms | < 50 ms | **not met** |
| G8 undo | 39 | 1.483 s | **0.301 s**, equal to the original in runs, every row, sections, statistics, warnings | < 1 s; equal | **met** |
| G8 memory (current RSS, F3) | 39 | peak growth (a wrong measure) | growth **11.6 MiB** (9.1 MiB at load 6); live data 24.4–34.6 MiB (§1.10.2 amended) | < 10 MiB | **not met** in this run (met at load 6) |
| G8 worst (a character in the common prefix) | 39 | 1,305 ms | **6.4 ms** | reported; ≤ G1's align phase | reported |
| `note_edit` in one section of two unrelated 1M-line files (F10) | 26 | ≈ 18 ms (review estimate) | median 0.014 ms, worst 0.026 ms | < 1 ms | **met** |

What still misses, and why. The edits that the certificates keep incremental now cost 5–17 ms at 1,000,000 lines
(from 25–45 ms): equality is one id comparison, counts are walks of the ids that stop early or are not needed, the rows
are spliced and the derivation is shared. The misses are the edits whose exact result restructures much of the
derivation: a 100-line shift moves diagonal-run pieces across the skew window's edge at several nesting levels (the
G1 pair's derivation nests about six levels of large nodes, each a 300-line window around a random walk of the
diagonal), so a paste re-certifies 130–450 nodes and moves many lines between their ends and middles; a copied section
or an Enter does it now and then. The work guard (note 28) sends such a recompare to the full compare once its work
passes an eighth of the lines, which bounds it at about the full compare (1.2–2.5 s under this load) instead of the
6–40 s measured without it. Mapping a gap that covers several old gaps to the largest recorded child among them was
tried and found inexact by P-EDIT-2 (reverted). `note_edit` passes 1 ms by up to 0.5 ms on the first edit after a
recompare and on a 100-line paste (the comparison's run and section vectors are copied once, then spliced): open. The
memory row depends on the allocator's reuse: 9.1 MiB at load 6, 11.6 MiB at load 39.

Re-measured after the split-gap fix (`f1a1edf`, load 7–9, the same harness): type median 12.8 ms, worst 15.6 ms
(met); changed median 8.9 ms, worst 14.6 ms (met); Enter median 10.3 ms, delete median 10.9 ms, each with one full
compare of 20 (worst 1.25 s, not met); paste median 1.36 s (every paste takes the full compare, not met); copy median
20.4 ms, worst 1.26 s (not met); undo 0.329 s and equal (met); memory growth 9.4 MiB (met); worst prefix edit 7.2 ms;
`note_edit` in one huge section worst 0.025 ms (met), elsewhere worst 0.72–1.43 ms (the 1 ms row not met on lines,
paste and copy).
