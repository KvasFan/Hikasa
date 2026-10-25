# Hikasa M1 — Binding API Contracts

> **Status: binding** for milestone M1 (version 0.1, `docs/16-roadmap.md`), branch `feat/m1-core-cli`.
> Implementers of waves 1–6 code strictly against this document, in parallel and without coordinating with
> each other. Every cross-crate type is defined exactly once, in the section of the crate that owns it, with
> its full signature. When this document and an older proposal disagree, this document wins.
>
> Approach: engines are pure, deterministic functions; the error model is explicit; invariants are stated
> and checked in code; property tests (`proptest`) and golden fixtures are designed into every crate; JSON and
> TOML shapes are stable; there is no hidden global state. Performance budgets are part of the contract
> (§0.8, §6.4).
>
> Sources: `docs/00`–`17`, `docs/reference/commands.md`, the root `Cargo.toml` (`[workspace.dependencies]`, the
> only third-party crates allowed) and every crate's `Cargo.toml`. Where `docs/` is ambiguous, this document
> records an interpretation in the decision register (§0.6). Each implementing PR copies the decisions it
> relies on into the `docs/*.md` file named there, as `17-agent-workflow.md` §1.2 requires (§0.9 lists them).

## Table of contents

- §0 Ground rules: scope, design tenets, conventions, code rules, dependencies per crate, decision register,
  dependency constraints, performance budgets, required documentation updates
- §1 Crate contracts: module trees, public API signatures, dependencies
  - 1.1 hikasa-core · 1.2 hikasa-vfs · 1.3 hikasa-diff · 1.4 hikasa-formats · 1.5 hikasa-text ·
    1.6 hikasa-folder · 1.7 hikasa-report · 1.8 hikasa-session · 1.9 hikasa-cli · 1.10 hikasa-gui (needs only) ·
    1.11 stub crates
- §2 Normative algorithms (spec parsing, masks, filters, timestamps, name alignment, folder states and counters,
  display filters, encoding, binary detection, tokenizer, regex engines, format detection, importance and keys,
  aligners, closeness, inline diff, rows/sections/statistics, text display filters, quick compare, binary
  compare, exit codes, configuration directory, IPC endpoint)
- §3 Data flows across crates
- §4 Stable JSON shapes (`--json`) and the IPC protocol
- §5 Concurrency, cancellation, progress, memory and error handling
- §6 Test plan, fixtures, benchmarks, performance gates, and acceptance criteria per implementation wave (1–6)
- §7 Requirement traceability
- Appendix A: built-in presets (complete C-family TOML, every other preset, complete keyword lists) ·
  Appendix B: invariant catalogue · Appendix C: open questions for the maintainer

---

## 0. Ground rules

### 0.1 M1 scope

| In scope (designed in full here) | Stays a stub, or out of scope |
|---|---|
| `hikasa-core`: Side, Spec (parse/display), errors, CancelToken, Progress and sinks, Job ids, the file-mask engine, NameFilter, ContentComparator, ContentKind, ExitCode | `hikasa-table`, `-hex`, `-picture`, `-metadata`, `-script` (crates stay empty) |
| `hikasa-vfs`: Entry model, the Vfs trait (with name-only prefiltered listings), LocalVfs, MemoryVfs, the stdin and clipboard blobs, the router, CRC32, binary compare, mmap (the only `unsafe` in library and program code; test and bench allocation counters are exempt, §6.1) | Archives, snapshots, remote back-ends, HTTP (these specs parse; resolving them reports `Unsupported`) |
| `hikasa-diff`: interner, DenseRemap, Unaligned/Standard/Myers/Histogram/Patience, closeness, never-align, inline diff, run model | Merge (3-way), alignment overrides (TC-013, FC-011) |
| `hikasa-formats`: model (TOML), grammar, tokenizer, regex engine selection, registry and shadowing, presets with complete keyword lists, encoding detection, binary detection, conversions None/Sorted/External | Spreadsheet/Word/RTF/PDF converters, syntect highlighting, table parsing |
| `hikasa-text`: TextDocument, TextRules, compare engine (identity fast path, progressive observer), rows, sections, stats, thumbnail, display filters, rules-based quick compare | Editing, undo, save, replacements (TC-023), "Ignored" marks (TC-024 marks), merge, patch apply |
| `hikasa-folder`: FolderRules (+ `criteria` vocabulary), parallel scanner filling a live `FolderModel`, per-folder counters, display filters, statistics, frozen `FolderComparison` | File operations, sync, merge, snapshots, automatic refresh, version compare (FC-028) |
| `hikasa-report`: text reports (side-by-side text/HTML, summary, patch normal/context/unified, statistics CSV), folder reports (side-by-side text/HTML, summary, JSON) | XML dump, interleaved, printing, links to file reports |
| `hikasa-session`: config paths, format registry loading, service API (incl. job spawning), legacy `gui open` argument parsing, IPC protocol, framing, endpoint naming, client and server | Sessions, workspaces, profiles, settings import/export, keychain |
| `hikasa-cli`: global flags; `quick`, `text compare`, `folder compare` (both with `--gui`, CLI-G03), `format`, `vfs`, `gui`, `completions`, `version` | Every other subcommand in `docs/12` (they are registered in M2+) |
| `hikasa-gui`: the contracts it needs (§1.10) and the wave-6 prototype outline (§6.5) | Implementation starts in wave 6 (Dioxus approved, D-56) |

### 0.2 Design tenets (normative)

| # | Tenet | How it is enforced |
|---|---|---|
| T1 | **Purity.** An engine result is a deterministic function of its explicit arguments. No ambient input (environment variables, current directory, clock, locale, thread identity, random numbers, `HashMap` iteration order, pointer values) may influence a result. | Ambient facts are captured once by an adapter (`hikasa_session::host::capture_env`, §1.8) and passed down as values. The architecture test (§6.1) bans `std::env::`, `SystemTime::now`, `Instant::now`, `thread::sleep`, `thread_local!` and `static mut` in `core/*/src`, except at exactly three allow-points (`ThrottledSink`, `StdProcessRunner`, `session::host`). Timeouts elsewhere use `interprocess` socket timeouts, `Receiver::recv_timeout` or `Condvar` waits; retry budgets are the sum of injected sleep durations. |
| T2 | **Parallel equals sequential.** Every function that accepts `Option<&rayon::ThreadPool>` returns the same canonical result for `None` and for a pool of any size. | Property tests run every parallel API with `None`, a 1-thread pool and an 8-thread pool, then compare canonical serialisations. |
| T3 | **Explicit error model.** Each crate has one `thiserror` error enum. Every error maps to an `ErrorKind` through the `Classify` trait. Each `ErrorKind`, combined with an operation context, maps to one exit code (§2.21). Cancellation is never an error state of a result model. | Unit tests per variant; exit-code table tests (§6.2.1, §6.2.8). |
| T4 | **Invariants are code.** Every result model has `check_invariants(&self) -> Result<(), InvariantViolation>`. Debug builds call it at construction (`debug_assert!`). Property tests call it on every generated case. | Appendix B lists every invariant with the test that covers it. |
| T5 | **Stable serialisation.** JSON and TOML shapes are owned by explicit DTO and config types, never by internal engine structs. Field order equals declaration order. Enums are `snake_case` strings. Maps are `BTreeMap` and never `HashMap`. Integers only (no floats). Timestamps are RFC 3339 UTC. Line numbers are 1-based. | Golden JSON files per command; `schema` field versioned (§4.1); TOML round-trip property tests for every persisted config type. |
| T6 | **Budgets, not deadlines.** Heuristic cut-offs (Standard alignment, closeness, regex backtracking) count operations, never wall-clock time, so results stay reproducible. Threads and timeouts may only affect *when* a call returns (cancellation latency), never *what* it returns. | Tests set tiny budgets and assert the documented fallback is taken. |
| T7 | **No hidden global state.** No lazily initialised mutable globals and no implicit rayon global pool in engines. Caches are owned by values, such as `FormatRegistry`'s per-entry `OnceLock`. Immutable `const`/`static` data is allowed. | Architecture test; global-pool trap (§5.1); review checklist. |
| T8 | **Warnings are data.** Core crates never log or print. They return `Vec<Warning>` inside results. Only `hikasa-session` logs, through `tracing`. | Architecture test bans `println!`, `eprintln!`, `print!`, `dbg!` and `tracing::` in `core/*/src` except `hikasa-session`. |
| T9 | **Totality.** Engines accept any bytes, any file name and any tree without panicking. Invalid configuration is rejected when it is compiled or parsed, never in the middle of a run. Inputs beyond documented limits are rejected with `TooLarge` before any engine that could assert on them runs (D-43). | `#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]` in every core crate. Property tests run with arbitrary inputs. |
| T10 | **CLI-first.** Every capability in this document is reachable from a `hikasa` subcommand (§1.9) and has a JSON shape (§4). | CLI integration tests (§6.3). |
| T11 | **One owner of every behaviour.** Vocabulary parsing (`--criteria`, `--unimportant`, `--display`, legacy `gui open` switches), exit-code mapping, display filtering and report rendering are core functions shared by the CLI, the GUI and (M2) scripts. Adapters only map flags to requests and results to output. | The vocabulary parsers have their own unit tests in core; CLI tests (§6.3) compare CLI results with direct `Service` calls; review checklist. |

### 0.3 Conventions used in this document

- Signatures are written to look compilable in Rust edition 2024. Bodies are elided with `;`. Private fields
  are shown only when they carry an invariant. The `///` lines are abbreviated here; the implementation
  documents every `pub` item in full (`#![deny(missing_docs)]`).
- Derive lists are part of the contract. `Serialize`/`Deserialize` on an engine type means it is persisted in
  TOML (rules, formats) or carried over IPC. It does **not** mean the type defines the CLI JSON shape; that
  belongs to DTOs (§4).
- Indices are 0-based in Rust. JSON and text reports use 1-based line numbers and 0-based byte offsets and
  row indices (§4.1).
- `Ctl<'a>` is the pair (cancellation token, progress sink), passed to every long operation (§1.1.9).
- MUST, SHOULD and MAY follow RFC 2119.
- "Wave N" refers to the implementation waves of §6.5.

### 0.4 Workspace-wide code rules

1. Edition 2024, `rust-version = "1.97"` (inherited from the workspace).
2. Every library crate root starts with:
   ```rust
   #![forbid(unsafe_code)]                 // hikasa-vfs: #![deny(unsafe_code)] (see 1.2), all others: forbid
   #![deny(missing_docs)]
   #![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
   ```
   `hikasa-vfs` cannot use `forbid`, because `forbid` cannot be relaxed. It uses `#![deny(unsafe_code)]`
   and puts `#[allow(unsafe_code)]` on exactly one item, `mod mmap;`. The current stub `lib.rs` has
   `#![forbid(unsafe_code)]`, and wave 2 changes it.
3. Library errors use `thiserror` 2. `anyhow` appears only in `cli/hikasa-cli/src/main.rs` (top-level context).
4. No `unwrap()`/`expect()` on data derived from user input or the filesystem. Panics inside jobs are caught
   at the service boundary (§5.6).
5. Core crates write only to `&mut dyn std::io::Write` values that callers pass in. They never write to
   stdout or stderr directly.
6. CI builds with `RUSTFLAGS=-D warnings`. Default trait-method bodies and stub functions name unused
   parameters with a leading underscore (`fn local_path(&self, _spec: &Spec)`), so no `unused_variables`
   warning can break the build.
7. Edition-2024 conventions: `std::env::set_var`/`remove_var` are `unsafe` in 2024 and are never used, not
   even in tests (tests build `EnvFacts` literally and pass environment to child processes with
   `Command::env`); `gen` is a reserved keyword, so no item is named `gen`; return-position `impl Trait`
   captures are written explicitly (`impl Iterator<Item = &str> + '_`).
8. Re-export policy (no extra manifest edges): `hikasa_folder` re-exports the `hikasa_vfs` types listed in
   §1.6 (`Attrs`, `Capabilities`, `DosAttrs`, `EntryKind`, `FollowLinks`, `UnixMeta`, `Vfs`, `VfsError`);
   `hikasa_text` re-exports the `hikasa_formats` and `hikasa_diff` types listed in §1.5. `hikasa-report` names
   format, diff and VFS types only through these re-exports; crates that already depend on the owning crate
   (`hikasa-session`, `hikasa-cli`) may use either path.
9. `#[non_exhaustive]` marks the error enums, `Spec`, `ErrorKind`, `WarningCode`, `Phase` and `JobKind`, which
   grow in later milestones. A `match` without a `_` arm is required wherever the matched enum belongs to the
   same crate (for example `ExitCode::for_content` over `ContentResult`), so that adding a variant forces a
   decision; matches over another crate's `#[non_exhaustive]` enum end with an explicit `_ => 100` arm.

### 0.5 Dependencies per crate (only `[workspace.dependencies]`)

Changes to a crate's current `Cargo.toml` are marked **+** (add), **−** (remove), **→dev** (move to
`[dev-dependencies]`) and **+dev** (add as a dev-dependency). Every change stays inside the approved workspace
list; no workspace-level feature changes are made. Every manifest row of the final workspace must match this
table (architecture test check 3).

| Crate | `[dependencies]` | `[dev-dependencies]` | Changes vs. current `Cargo.toml` |
|---|---|---|---|
| hikasa-core | serde, thiserror, bitflags (declared; no M1 type uses it), **jiff**, **unicode-normalization** | proptest, pretty_assertions, **regex** (mask oracle), **serde_json**, **criterion** | + jiff (RFC 3339 for `FileTime`); + unicode-normalization (NFC); +dev regex, serde_json, criterion (mask bench) |
| hikasa-vfs | hikasa-core, serde, thiserror, crc32fast, memmap2, filetime (tests; M2 touch), bitflags, same-file | tempfile, proptest, **pretty_assertions**, **criterion** | lib.rs: `forbid` becomes `deny(unsafe_code)` plus one `allow` on `mod mmap`; +dev pretty_assertions, criterion (binary/CRC benches) |
| hikasa-diff | hikasa-core, imara-diff, serde, thiserror, smallvec | proptest, pretty_assertions, criterion, **similar** (oracle), **serde_json** | `similar` →dev (used only as an independent LCS oracle); +dev serde_json (golden files) |
| hikasa-formats | hikasa-core, serde, toml, regex, fancy-regex, encoding_rs, chardetng, thiserror, memchr, **rayon** | proptest, pretty_assertions, tempfile, **criterion**, **serde_json** | + rayon (parallel speculative tokenisation, §2.10.6); − indexmap (no code depends on TOML key order, D-41); +dev criterion, serde_json |
| hikasa-text | hikasa-core, hikasa-diff, hikasa-formats, hikasa-vfs, ropey, serde, thiserror, unicode-segmentation, memchr, **rayon**, **toml**, **bitflags**, encoding_rs (chunked, cancellable decoding, §1.5.7), regex (`regex::escape` of plain unimportant-text patterns, §1.5.7); declared but unused in M1: unicode-width | proptest, pretty_assertions, tempfile, criterion, **serde_json** | + rayon (parallel keys); + toml (`TextRules` files); + bitflags (`TextToggles`, `ThumbCell`); +dev serde_json |
| hikasa-folder | hikasa-core, hikasa-vfs, rayon, serde, thiserror, unicode-normalization (declared but unused in M1: NFC folding comes from `hikasa_core::case`), crossbeam-channel, parking_lot, **jiff**, **bitflags**, **toml** | tempfile, filetime, proptest, pretty_assertions, **criterion**, **serde_json** | + jiff (date filters relative to local midnight); + bitflags (`DiffFlags`, `NodeFlags`, `FolderToggles`); + toml (`FolderRules` files); +dev criterion, serde_json |
| hikasa-report | hikasa-core, hikasa-diff, hikasa-text, hikasa-folder, serde, serde_json, jiff, thiserror, unicode-width, **bitflags** | pretty_assertions, tempfile, **diffy** (patch-apply oracle), **proptest**, **hikasa-vfs** (`MemoryVfs` fixtures) | + bitflags (`FolderColumns`); +dev diffy, proptest, hikasa-vfs. No normal edge to hikasa-formats or hikasa-vfs: rule 8 of §0.4 supplies the types |
| hikasa-session | hikasa-core, -vfs, -diff, -formats, -text, -folder, -report, serde, serde_json, toml, directories, interprocess, thiserror, tracing, **rayon**, **jiff**, **parking_lot**, **crossbeam-channel** | tempfile, pretty_assertions, **proptest** | + rayon (owns the explicit `ThreadPool`); + jiff (clock and zone values); + parking_lot (job registry, IPC waiters); + crossbeam-channel (`JobHandle` channels); +dev proptest |
| hikasa-cli | the M1 list without `anyhow` | unchanged list | − anyhow (wave 5: every error is an `ApiError` routed through the session envelope helpers, so the binary needs no generic error type). `clap_complete`, `tracing-subscriber` and `jiff` are used as listed |
| hikasa-gui | hikasa-core, hikasa-vfs, hikasa-formats, hikasa-text, hikasa-folder, hikasa-report, hikasa-session, **dioxus** (features `desktop`), anyhow, serde, serde_json, tracing, tracing-subscriber, parking_lot, crossbeam-channel | tempfile, pretty_assertions | wave 6: + dioxus 0.7 (D-56, approved) and the crates the views render; anyhow only in `main.rs` |

Rejected inside M1, although they are in the workspace list: `blake3` (content caches are P1, VFS-020; endpoint
hashing uses an in-house FNV-1a, §2.23), `similar` in non-test code (one LCS implementation, imara-diff, keeps
behaviour single-sourced), and `diffy` in non-test code (patch apply is P1, TP-002).

### 0.6 Decision and interpretation register

Each decision is binding for M1. The "Docs" column names the file that must receive the text (§0.9).

| ID | Decision | Rationale | Docs |
|---|---|---|---|
| D-01 | `hikasa quick` and `hikasa text compare` exit codes: 1 bytes identical, 2 rules-based same, 11 binary content that differs (§2.17.5), 12 only unimportant differences (2 with `--ignore-unimportant`), 13 important differences. **Any** error, including an input that cannot be loaded, gives 100. | One mapping for every file comparison. 107 is reserved for a base folder that cannot be loaded (CLI-004 and the M1 contract); a file input is not a base folder. | 12 §2 |
| D-02 | `hikasa folder compare`: 13 if any important difference; else 100 if the result is incomplete or any node has state Error or Unknown (an incomplete result cannot claim sameness); else 12 if only unimportant differences; else 2. Base not loadable: 107. | A comparison that could not inspect everything must not return "no differences"; differences found are still reported as 13. | 12 §2 |
| D-03 | Display filters (`--display`, `--context`) never change the exit code or the summary. `--ignore-unimportant` does change them, because it changes the meaning of "same". | Exit codes describe data, not presentation. | 12 §2 |
| D-04 | Default `TextRules`: leading, embedded and trailing whitespace **important**; "everything else" important and case-sensitive; every grammar element important unless the format's `ElementInfo::important` says otherwise (the Python `Whitespace` element and the COBOL `Identification` element ship unimportant, Appendix A); no unimportant-text patterns; *orphan lines are always important* ON; *compare line endings* OFF (TC-022); encoding and BOM are never compared; algorithm `Auto`; skew 300; closeness ON (threshold 500‰); never-align OFF. With these defaults a comparison is exact apart from line endings, encoding and BOM, and the identity fast path (§2.13.4) applies. Ignoring whitespace is an explicit opt-in (`--unimportant whitespace`). | A plain `text compare` must not hide whitespace-only edits; exact defaults keep the 1M-line gate cheap. | 05 §3 |
| D-05 | Recognised but unsupported schemes (ftp, ftps, sftp, dav, davs, s3, svn, svn+http(s), svn+ssh, mtp, profile:, http(s)) **parse successfully**. Resolving them in the VFS fails with `VfsError::Unsupported`. An unknown `scheme://` fails to parse with `SpecError::UnknownScheme`. A single-letter scheme on Windows is a drive letter. | Specs round-trip (ARCH-020), so sessions written by later versions remain readable. An unknown scheme must never be silently reinterpreted as a local path. | 11 §2 |
| D-06 | Local path normalisation: `.` and duplicate separators are removed. On Unix, `..` is **kept** (lexical removal is unsound across symlinks), except that `/..` collapses to `/`. On Windows, `..` is resolved lexically, as the OS itself does. | Correctness with symlinks. | 11 §2 |
| D-07 | Masks: `\` and `/` are both separators; a trailing separator marks a folder mask. Case-insensitive matching uses the 1:1 simple lower-case mapping (`hikasa_core::case`). Names and masks are compared in NFC. A trailing `.` means "no extension". A mask ending in `.*` also matches names without an extension. A leading `-` excludes; `[-]` matches a literal `-`. | FC-051 plus explicit normalisation. | 04 §6.1 |
| D-08 | A relative mask (`p\f`) is anchored at the base folder. `.\f` matches in the base folder only. `...` is a path component that matches zero or more folders, anywhere in the mask. A mask without separators matches the item name at any depth. | FC-051. | 04 §6.1 |
| D-09 | An include-folder mask includes the whole subtree of a matching folder. Folders that are only *prefixes* of a relative include mask are traversed but not themselves "included". The base folder is always included. | Matches the user intent of `src\`. | 04 §6.1 |
| D-10 | File-format masks match the file **name** only and are always case-insensitive. | `*.C` (C++ on Unix) must still be detected. | 10 §1 |
| D-11 | M1 behaviour of non-Text kinds: `Table` (CSV) is compared as plain text with the "Everything Else" grammar. Picture, Media, Version and Registry kinds use binary comparison. | Their engines are stubs in M1. | 10 §2 |
| D-12 | Content classified binary on either side (§2.9) never gets a rules-based verdict. In every rules-based path — `text compare`, `quick --rules` and `RulesComparator` — the verdict is `BinarySame` (bytes identical) or `BinaryDifferent` (exit 1 / 11). `text compare` still builds rows for display and adds warning `binary_content`. | Rules-based semantics are undefined for binary data; all paths agree (TC-016). | 04 §3, 05 §1 |
| D-13 | Whitespace classes (leading, embedded, trailing) apply only to text that is not covered by a grammar token. A token piece created by line continuation excludes the line's leading-whitespace prefix. A token piece ended by end-of-line excludes the trailing-whitespace suffix. | Re-indenting a block comment must stay unimportant when leading whitespace is unimportant; Python's explicit `Indent` item still wins. | 10 §3 |
| D-14 | Line keys are built from atoms (§2.13). Grammar tokens are atomic: `int x` ≠ `intx`. "Everything else" text concatenates across removed whitespace, like `diff -w`: in plain text `a b` equals `ab` when embedded whitespace is unimportant. | Avoids both false "unimportant" results on code and false "important" results on prose. | 05 §3 |
| D-15 | One **effective format** per text comparison: the forced format (`--format`); else detection on the first side that has a file name (left, then right); for a stdin side the name is the `--stdin-name` hint. When both sides have names and their detections differ, warning `format_mismatch` is emitted. No name at all → "Everything Else". | Keys from two different grammars are not comparable; `hikasa text compare - x.c` must still use the C grammar. | 05 §11 |
| D-16 | Standard alignment = staged anchors (unique, weighted LIS → best block → trivial block) in a two-diagonal skew window, with a deterministic work budget. When the budget runs out, each remaining range is aligned by Histogram only while its estimated cost `|A|·|B|` fits a second deterministic budget, the fallback budget (128·(n+m) + 32,000,000 cells); beyond it the range is split once at lines unique on both sides (patience LIS) and its gaps are paired per §2.15, with warning `alignment_degraded` (§2.14.5; amended in wave 2, §1.3.3 item 3). Standard is O(n log n) plus its budgets on every input. Explicitly chosen Histogram, Myers and Patience stay exact (never degraded, no budget): their adversarial worst case is super-linear, and they stay cancellable. Every imara-diff call on a (sub-)range goes through `DenseRemap` (§2.14.3). | Bounded time with reproducible output; imara-diff is roughly quadratic on adversarial input and allocates O(max token id) per call. | 05 §2 |
| D-17 | Closeness matching (§2.15) is applied to the gaps of Standard, Histogram and Patience. Unaligned pairs by position. Myers pairs by position and, when closeness is on, adds warning `closeness_ignored` (TC-010: Myers has no closeness). | Source code (Auto = Histogram, D-36) gets changed lines with inline highlights (TC-011). | 05 §2 |
| D-18 | Statistics (RPT text "Statistics") are counted per section: `added`/`deleted` for one-sided sections, `original`/`changed` for two-sided ones. The I/U class is the section's importance (§2.17.3). | Gives the normal-diff `a`/`d`/`c` semantics. | 13 |
| D-19 | In folder display filters, Unknown and Error nodes belong to the atomic category *Differences*. | Conservative: they surface when you look for problems. | 04 §6.3 |
| D-20 | Entries of different kinds (file, folder, symlink-not-followed) with the same name are **not** aligned. They appear as two orphan rows, folder first. | Different kinds take different operations. | 04 §2 |
| D-21 | Read-only CLI commands never write the built-in formats to the config directory. `FormatStore::materialize` (FF-004) runs only on GUI first start and on explicit `format` write commands (M2). | 00 §4 safety: never write files the user did not target. | 10 §1 |
| D-22 | `hikasa gui ping\|list\|close\|quit` never launch the GUI. When no GUI is running they exit **100** ("not running"). Only `gui open` and `--gui` (D-50) launch it. | Pinging should not start applications; 102 means "unable to wait" (CLI-004). | 12 §1.1, §2 |
| D-23 | `gui open --wait` (and `--gui --wait`) exit code = the view's code when it closes (1/2/11/12/13 as for `quick`, 2/12/13/100 for folder views; 0 if the view has no verdict; 100 if the view failed to load). **102** = the CLI cannot wait for the GUI: the connection is lost while waiting, the launched GUI never became reachable within the retry budget, or `--wait --no-launch` finds no GUI. Without `--wait`, `--no-launch` with no GUI gives **100**. **103** = no GUI executable found. | What difftools need (CLI-010), and CLI-004's meaning of 102. | 12 §2b |
| D-24 | Engines never read the clock. Report "generated at" and date filters use values that the adapter injects. | T1. | 13 |
| D-25 | Content tests in a folder comparison acquire `min(size_l + size_r, budget)` bytes from a shared `ByteBudget` before running and release them afterwards (default budget 1 GiB; `options.toml` key `[tweaks.file_operations] content_memory_budget_mib`). A comparator reports `size_limit()`; `RulesComparator` (default limit 128 MiB per file) compares larger files by bytes, adds warning `rules_size_limit`, and the node gets flag `RULES_SKIPPED_SIZE`. Quick and text compare of a single pair have no cap beyond the document limits (D-43). | Memory bounded independently of the thread count (§5.7). | 04 §3, 15 |
| D-26 | The CSV preset mask is `*.csv;*.tsv`. The doc's `*.txt?` entry is dropped as a typo. | Avoids silently capturing `*.txt`. | 10 §5 |
| D-27 | The built-in "Registry Files" format ships **disabled** (`Preset::enabled = false`; it is not a field of the format TOML). Otherwise it is fully shadowed by "Registry Dumps / INI" (`*.reg`), and the registry view is P1. | No red (shadowed) built-in on first run; enabled flags live only in `order.toml` (SES-051). | 10 §5 |
| D-28 | Timestamps are compared at nanosecond resolution; tolerance is inclusive. With *ignore time zone*, `k` multiples of 1 h are tried with \|k\| ≤ 26, which covers every real UTC offset pair (−12…+14). | FC-021. | 04 §3 |
| D-29 | Folder JSON is a flat, pre-order list of visible items with `path` and `depth` (not nested objects). | Streams well, works with `jq`, gives stable diffs. | 12 |
| D-30 | The DOS attribute letter order is `A C D E H I L M N O P R S T U Z`. `P` is pinned, `U` is unpinned, `M` is recall-on-data-access, and `Z` is sparse. `DosAttrs` serialises everywhere (TOML, JSON, IPC) as its canonical letter string, e.g. `"HRS"`, through hand-written serde impls (not bitflags' flag-name form). | FC-040 lists the letters, not their meaning; one textual form. | 04 §5 |
| D-31 | TOML config files: a `schema_version` newer than supported is an error (SES-052). Format and rules files: an unknown key produces warning `unknown_config_key`. `options.toml`: unknown keys inside the top-level sections of docs/15 are ignored silently (M2 writes the whole file); an unknown top-level section produces `unknown_config_key`; with `--verbose` every ignored key is logged (tracing, not a warning). | Forward compatibility without warning floods, and without silently dropping typos in user-authored files. | 03 §5 |
| D-32 | `hikasa vfs cat --json` returns the content as base64 inside the JSON envelope. | CLI-000 says every subcommand supports `--json`. | 12 |
| D-33 | Following symlinks: loops are detected by `FileId` (Unix dev+ino; Windows `same-file` handles). A loop yields an Error node with flag `SYMLINK_LOOP`, and depth is capped at 256. | FC-013. | 04 §2 |
| D-34 | `skip_if_quick_same` (FC-026) defaults to **off**: when content comparison is requested, it runs. | Correctness-first default; the docs give none. | 04 §3 |
| D-35 | clap usage errors exit with **100**, not clap's default 2 (2 means "rules-based same"). Panics are caught and exit with **100**, not Rust's default 101 (101 means "conflicts, output not written"). A broken pipe on stdout stops output and keeps the computed exit code. | Exit codes must never lie. | 12 §2 |
| D-36 | `AlgorithmChoice::Auto` = Histogram when the effective format has a non-empty grammar ("source code"), Standard otherwise. *Amended in wave 3 (review decision 4):* for source code, `Auto` is Histogram only while the estimated cost of its stripped middle, `|A|·|B|` cells after removing the common prefix and suffix of the interned key sequences, fits hikasa-diff's fallback budget `AlignOptions::fallback_budget_for(n, m)` (128·(n+m) + 32,000,000, §1.3.3 item 3); beyond it `Auto` is the bounded Standard (with line weights). Only lengths decide (deterministic, T6); `TextComparison::algorithm` reports the engine used. Explicitly chosen Histogram, Myers and Patience are unchanged (D-16). | TC-010; an uncancelled run of `Auto` must stay bounded (repetitive 2 × 7 MiB C files: 9.2 s with Histogram, 0.2 s now). | 05 §2 |
| D-37 | Size units in filters are binary: KB = 1024, MB = 1024², GB = 1024³, TB = 1024⁴. | Common convention for file tools. | 04 §6.2 |
| D-38 | The Sorted conversion sorts lines by Unicode code point (stable sort). Each output line gets the document's dominant line ending, and the document ends with an EOL only if the original did. A sorted document is read-only. | FF-030. | 10 §4 |
| D-39 | External conversion command lines are split into argv by Hikasa's own quoting rules and run **without a shell** (§2.10.8). | Deterministic and safe against injection. | 10 §4 |
| D-40 | When `--json` is given, JSON goes to stdout; progress and logs go to stderr; `--quiet` suppresses both progress and non-error stderr output. Human output is one record per line where applicable (CLI-000). | CLI-000. | 12 §1 |
| D-41 | `ElementId`s are dense: 0 = "Everything Else"; then the elements in order of first use in the `grammar` array; then elements declared in `[elements]` but never used, in lexicographic name order. `[elements]` is an unordered settings map (`BTreeMap`). | `toml` 1.x without `preserve_order` deserialises every table through a `BTreeMap`; arrays keep their order. No workspace feature change is needed. | 10 §3 |
| D-42 | Encoding detection order (§2.8): override → BOM → Fixed → BOM-less UTF-16/32 pattern → pure ASCII (format fallback, then the injected locale encoding, each only if ASCII-compatible, else UTF-8) → valid UTF-8 → mostly valid UTF-8 (at least one valid multi-byte sequence, at most 16 malformed sequences and at least 8 valid multi-byte sequences per malformed one: decoded as UTF-8 with U+FFFD, source `utf8_lossy`, warning `lossy_decode`) → chardetng guess (UTF-8 denied, ISO-2022-JP denied). One heuristic function serves detection and binary sniffing; it accepts a zero-byte pattern (or BOM-less UTF-16 in one non-Latin alphabet) only when the decoded sample is plausible text (≤ 10 % control characters). chardetng 1.0's `find_score` is a test-only API, so a format fallback is used only for content without non-ASCII evidence. *Amended in the wave-2 review:* one stray byte in a UTF-8 file no longer re-decodes the whole file in a legacy encoding, and arrays of small integers are no longer text. | ASCII-range UTF-16 is valid UTF-8, so the pattern test must come first; docs order "BOM → chardetng → format default → locale". Valid multi-byte UTF-8 is strong evidence: legacy text almost never forms it. | 05 TC-001, 10 FF-032 |
| D-43 | Document limits: at most 4 GiB − 1 source bytes and at most `i32::MAX − 2` lines (imara-diff asserts `len < i32::MAX`). Larger inputs are rejected with `TextError::TooLarge` / `TooManyLines` when the line table is built, before any alignment. | T9: no user input may reach an assertion. | 05 §1 |
| D-44 | *Suppress Filters* (FC-057) reveals items hidden by **file filters** (name and other filters): they become visible in teal. Display filters (FC-055) keep applying to every other item. | FC-030/FC-031/FC-054 tie Suppress Filters to file filters. | 04 §6.3 |
| D-45 | Entries excluded by **name masks** on a side are listed *name-only* when the back-end can tell their kind from the directory record (no `stat`): they carry name and kind, size 0, no times and no attributes, and `stat_skipped = true`. They still appear as filtered (teal) items under Suppress Filters; the filtered side of an FC-054 pair (its counterpart is visible) is stat'ed after alignment. Entries hidden by size, date or attribute filters are stat'ed normally. Symlinks and entries of unknown kind are always stat'ed. | Saves the dominant scan cost (`stat`) for excluded names without losing FC-054/FC-057. | 04 §6 |
| D-46 | IPC forward compatibility: `Request`, `Response` and `ViewKind` have a `#[serde(other)] Unknown` variant; frames are decoded in two phases (`serde_json::Value`, then `id`, then the body), so an unknown `type` is answered with `Error{unsupported_request}` carrying the original id; error codes and view results travel as plain strings. | ARCH-091: newer peers must not break older ones. | 01 ARCH-091 |
| D-47 | IPC endpoint per user **and configuration**: `hikasa.sock` / `\\.\pipe\hikasa-<user>`, with a `-<h8>` suffix (FNV-1a-64 of the absolute config dir) whenever the config dir is not the platform default; long Unix socket paths fall back to `hk-<h16>.sock`; the single-instance lock sits next to the endpoint (§2.23). | Portable and `--config-dir` GUIs neither collide nor share a lock. | 01 ARCH-090/091 |
| D-48 | *Override quick test results* (FC-027) lets a content match (`BinarySame`, `RulesSame`) cancel only the `TIME` and `SIZE` quick-test differences. `NAME_CASE`, `ATTRIBUTES`, `PERMISSIONS`, `OWNER`, `GROUP` and `LINK_TARGET` differences still make the pair `Different`. | FC-027 says "content match wins over timestamp mismatch"; metadata criteria the user enabled must not be hidden. | 04 §3 |
| D-49 | `gui open -` spools stdin to a file in the cache directory (`<cache>/stdin/`, named from `--stdin-name` or `stdin.txt`) and sends that absolute path; with `--wait` the CLI deletes the file after the view closes. `clipboard://` is rejected by `gui open` and `--gui` in M1 (exit 100), because specs sent over IPC must be absolute and there is no payload transfer. | The GUI has another cwd and no access to the CLI's stdin. | 12 §1.1 |
| D-50 | `--gui` on `text compare` and `folder compare` (CLI-G03) sends the same specs, rules file paths, rules patches and view options through `Service::gui_open`, instead of comparing locally. `--wait` and `--no-launch` apply as for `gui open`. | CLI-G03 is P0. | 12 §1.1 |
| D-51 | The legacy `gui open` argument handling (CLI-001 spellings, CLI-002 positional forms, CLI-003 switches) lives in `hikasa_session::api::gui` (`normalize_legacy_args`, `parse_open_args`); `hikasa gui open` and `hikasa-gui` call the same functions. | ARCH-L3: the GUI must not duplicate CLI logic. | 12 §2 |
| D-52 | `TimestampRule` has an explicit `enabled` flag (no `Option`, because TOML has no null), plus an optional `offset: TzOffset` (shift one side by whole hours, the script `timezone:left+N` criterion). | FolderRules must round-trip through TOML. | 04 §3 |
| D-53 | Identity fast path: when the rules make every char of every line important and case-sensitive (D-04 defaults, and no unimportant element in the format), a line's key is its raw text (prefixed by its line-ending code when line endings are compared) and no tokenisation runs. Results are identical to the general path. *Amended in wave 3 (review decision 2):* the identity key is the raw text followed by the end markers of §2.13.2 (`ART`, or `EOL kind` when endings are compared), and the general key emits token boundaries and element ids only where an unimportant char was actually dropped (§2.13.2), so the general key of a line from which nothing is dropped or folded is byte-identical to its identity key, whatever the tokenizer state. The equivalence (INV-KEY-3) therefore holds for every grammar, multi-line items included, and an unimportant-text pattern that never matches does not change a result (P-TC-7). D-14 is kept: `int x` ≠ `intx` under ws-off. | The default rules must be cheap; results must not depend on an internal switch. | 05 §3 |
| D-54 | Long imara-diff calls (more than 4,096 tokens on either side; 200,000 before wave 2, §1.3.3 item 1) run on a helper thread; the caller polls its `CancelToken` every 10 ms and returns `Err(Cancelled)` without waiting for the call (the result is discarded). imara-diff cannot be interrupted, so the helper of a cancelled call runs until its call ends; at most 2 such abandoned helpers run per process, and a new helper call waits (polling its `CancelToken`) while 2 are running (§1.3.3 item 2). | ≤ 100 ms cancellation latency without changing results (T6); bounded CPU after repeated cancellations. | 05 §2 |
| D-55 | Folder aggregates are **counters**: every folder holds per-category counts of its descendants (files and folders separately), for both values of ignore-unimportant, maintained incrementally (`dec(old)/inc(new)` on every ancestor). Folder state, colour, visibility, statistics and verdict are O(1) per folder, on partial trees too. | Live model for the GUI (ARCH-004, FC-016), O(1) Ignore-Unimportant toggling (FC-057). | 04 §4 |
| D-56 | The GUI toolkit is Dioxus 0.7 with the `desktop` feature (ARCH-GUI-001). **Approved by the maintainer for wave 6** and added to `[workspace.dependencies]`; the ARCH-GUI-003 performance gate still decides whether the heavy views stay on it. | The maintainer proposed Dioxus; no new dependency without approval (17-agent-workflow §2.6). | 02 |
| D-57 | `--criteria <list>` (folder compare) follows the script `criteria` syntax (SCR): items separated by `,` or spaces; listed criteria are on, unlisted quick tests and the content method are off. Hikasa adds `filename-case` (FC-022). `version` (FC-028, P1) is rejected in M1. | One vocabulary for CLI and scripts (T11). | 12 §3.1 |
| D-58 | A format may declare `word_chars` (extra characters that count as word characters for List word boundaries and keyword lookup). COBOL uses `"-"`. | `END-IF` is one keyword; `END-OF-FILE` is one identifier. | 10 §3 |

### 0.7 Dependency constraints (consequences of the approved list)

The following crates are **not** in `[workspace.dependencies]`, so M1 does without them. These are
consequences, not decisions to revisit inside M1; adding any of them needs the 17-agent-workflow §2.6 process.

| Missing crate | Consequence in M1 |
|---|---|
| `tokio` | IPC is blocking: one accept thread plus one thread per connection (ARCH-080's tokio runtime is deferred). |
| `ignore`, `globset` | In-house folder walker (`hikasa-folder` scanner) and mask engine (`hikasa_core::mask`). |
| `notify` | No file watching: FC-017 automatic refresh, TC-006 and SES-053 change detection are M2+. |
| `arboard` | `clipboard://` resolves only when an adapter injects a `Blob`; the CLI reports `Unsupported`; `gui open clipboard://` is rejected (D-49). |
| `infer` | Magic-number detection is an in-house table (§2.9). |
| `libc`, `thread-priority` | ARCH-061 comparison priority cannot be applied (gap, Appendix C); owner and group are shown as numeric ids; the effective uid is obtained with std only (`capture_env`, §1.8). |
| a signal-handling crate | Ctrl-C terminates the CLI (no graceful cancel); every `--output` file is written atomically, so no partial report appears. |
| `dioxus` | The GUI (wave 6) needs approval first (D-56). |
| `tempfile` (dev-only) | Atomic writes are hand-rolled (`AtomicWrite`, `write_atomic`). |

Feature facts of approved crates that shape the design: `toml` has no `preserve_order` (D-41); `serde_json` has
no `preserve_order` (JSON produced from structs keeps declaration order, `serde_json::Value` objects sort keys);
`jiff` has no `serde` feature (`FileTime` has hand-written serde); `ropey` keeps its default Unicode line
breaks, so Hikasa keeps its own line table (§1.5.2); chardetng 1.0 exposes `find_score` only behind a test-only
feature (D-42).

### 0.8 Performance budgets (release build, 2020-class 8-thread laptop, NVMe SSD, headless)

**Text compare of two 1,000,000-line files** (≈ 48 bytes per line, about 1 % of the lines changed in clusters);
contract < 3 s, design target < 1.5 s:

| Phase | Work | Time budget | Peak memory added |
|---|---|---|---|
| read | 2 × 48 MB through `Vfs::open_read` into exact-size buffers | ≤ 100 ms warm | the raw bytes (dropped after decoding) |
| decode + line table | UTF-8 validation or decoding, rope build, `memchr` line split | ≤ 150 ms | rope ≈ 1.05 × text, line table 8 B/line |
| keys | identity fast path: raw lines into per-chunk arenas; grammar path: `tokenize_all` in parallel 8192-line chunks | ≤ 100 ms identity, ≤ 400 ms grammar | arenas ≤ 1 × text (transient) + 8 B/line; *wave 3:* a key is the kept text plus markers (§2.13.2), so ≈ 1 × text on both paths (G1: 294 MiB total peak); worst case ≈ 2 × text + 2 B/line for token-dense lines whose separating whitespace is removed (`a b c` under ws-off), 3 × with more than 127 elements |
| intern | sequential hash interning, left then right | ≤ 150 ms | 4 B/line ids + one hash-table entry per distinct key: 28–57 B, ≈ 30 B on this input (§1.3.3 item 11) |
| align | Histogram ≤ 300 ms; Standard ≤ 1 s | | O(n + m) |
| pair + classify | closeness DP inside its budgets (cells in hikasa-diff; scorer work units in hikasa-text, §2.15.2), rows, sections, statistics | ≤ 200 ms (*wave 3:* the scorer budget is ≈ 0.1 s of work on the worst input) | rows 12 B/row |
| **total** | | **< 3 s** | **≤ 3 × input bytes + 64 B/line + 32 MiB** |

**Folder compare, 100,000 files per side** (10,000 folders, depth ≤ 8); contract < 5 s cold, < 1.5 s warm:

| Phase | Work | Time budget | Memory |
|---|---|---|---|
| listing | 2 × 100k entries, `read_dir` + metadata, directory pairs in parallel; excluded names are not stat'ed (D-45) | ≤ 3 s cold, ≤ 0.5 s warm | per-directory batches, freed after insertion |
| align + quick tests | per directory, O(k log k) | ≤ 100 ms | — |
| model | contiguous children, per-side entry tables | ≤ 50 ms | ≤ 256 B per aligned pair plus name bytes |
| counters | O(depth) per directory batch and per content result | ≤ 20 ms | 2 × 18 × 4 B + 48 B per folder |

**Binary compare** streams at ≥ 90 % of the sequential-read throughput of the same files. **Cancellation**
returns within 100 ms. **GUI enablers** on the 1M-line result: `TextComparison::row` and `FilteredView::get`
< 200 ns, `TextComparison::inline` p99 < 50 µs. The gates are in §6.4.

### 0.9 Required documentation updates

Each wave's PRs carry these doc edits (17-agent-workflow §1.2, §4.1). New requirement IDs are added rather than
renumbering existing ones.

| Wave | File | Content |
|---|---|---|
| 1 | `docs/02-tech-stack.md` | §0.7 dependency constraints; interprocess/chardetng/toml feature notes; "mmap residual risk" |
| 1 | `docs/04-folder-views.md` §6.1 | D-07, D-08, D-09 |
| 1 | `docs/11-vfs-remote-archives.md` §2, §3 | D-05, D-06; mmap re-stat mitigation (§2.20) |
| 2 | `docs/10-file-formats.md` | D-10, D-26, D-27, D-38, D-39, D-41, D-42, D-58, `ElementInfo::important`, magic numbers (§2.9) |
| 3 | `docs/05-text-views.md` | D-04, D-12–D-17, D-36, D-43, D-53, D-54 |
| 3 | `docs/04-folder-views.md` §2–§6 | D-19, D-20, D-25, D-28, D-33, D-34, D-37, D-44, D-45, D-48, D-52, D-55 |
| 4 | `docs/13-reports.md` | D-18, D-24, RPT-004 header fields (§1.7) |
| 4 | `docs/03-sessions-settings-profiles.md` §5, `docs/15-options-reference.md` | D-31, cache directory (§2.22), `content_memory_budget_mib` (D-25) |
| 4 | `docs/01-architecture.md` §5 | D-46, D-47; ARCH-061 gap |
| 5 | `docs/12-cli-and-scripting.md` | D-01, D-02, D-03, D-22, D-23, D-29, D-32, D-35, D-40, D-49, D-50, D-51, D-57; the M1 flag set of §1.9.2 |
| 6 | `docs/02-tech-stack.md` | GUI toolkit approval (D-56) and the ARCH-GUI-003 measurements |

---

## 1. Crate contracts

### 1.1 hikasa-core

**Purpose.** Shared vocabulary. This crate depends on no workspace crate (ARCH-010). It does no I/O except
through the `ContentSource` trait objects that callers pass in.
**Uses.** serde, thiserror, jiff, unicode-normalization (bitflags stays declared; no M1 type uses it).

#### 1.1.1 Module tree

```
core/hikasa-core/src/
├── lib.rs            crate docs; `pub mod` list; `prelude` re-exports
├── side.rs           Side, Pair<T>
├── spec/
│   ├── mod.rs        Spec, SpecContext, PathStyle
│   ├── local.rs      LocalPath + pure, style-explicit normalisation (§2.1)
│   ├── url.rs        RemoteUrl, RemoteScheme, HttpUrl, Secret
│   └── error.rs      SpecError
├── path.rs           RelPath (normalised relative path, '/' separated)
├── case.rs           simple_lower, fold, eq_folded, nfc  (normative for all crates)
├── mask/
│   ├── mod.rs        Mask, MaskList, MaskOptions, MaskTarget, Polarity, CaseSensitivity
│   ├── glob.rs       component-pattern compiler + matcher (no regex)
│   └── error.rs      MaskError
├── filter.rs         NameFilter, NameFilterLists, FolderVerdict
├── content.rs        ContentSource, ReadError, ContentComparator, ContentResult, ContentKind, ContentMethod,
│                     ContentOutcome
├── cancel.rs         CancelToken, Cancelled
├── progress.rs       Progress, Phase, Unit, ProgressSink, NullProgress, ThrottledSink, Ctl (lazy reports)
├── job.rs            JobId, JobKind, JobIdAllocator
├── time.rs           FileTime, TimeParseError
├── exit.rs           ExitCode, CompareVerdict
├── error.rs          ErrorKind, Classify, ErrorReport, InvariantViolation, io_error_kind
├── warning.rs        Warning, WarningCode
└── schema.rs         check_schema_version, SchemaCompat, SchemaError
```

#### 1.1.2 `side`

```rust
/// One side of a comparison (00-overview §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// Left input.
    Left,
    /// Right input.
    Right,
    /// Ancestor of a 3-way merge (reserved in M1).
    Center,
    /// Merge target (reserved in M1).
    Output,
}

impl Side {
    /// The two input sides of a 2-way comparison, in canonical order.
    pub const INPUTS: [Side; 2] = [Side::Left, Side::Right];
    /// Left↔Right; `Center` and `Output` map to themselves.
    pub const fn opposite(self) -> Side;
    /// Stable lowercase name, identical to the serde form ("left", …).
    pub const fn as_str(self) -> &'static str;
}

impl fmt::Display for Side { /* writes `self.as_str()`; used by error messages such as `TextError::Load` */ }

/// One value per input side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Pair<T> {
    /// Left value.
    pub left: T,
    /// Right value.
    pub right: T,
}

impl<T> Pair<T> {
    /// Builds a pair.
    pub const fn new(left: T, right: T) -> Self;
    /// The value of `side`; `None` for `Center` and `Output`.
    pub fn get(&self, side: Side) -> Option<&T>;
    /// Maps both values (left first: the order is observable when `f` has side effects).
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Pair<U>;
    /// Borrows both values.
    pub fn as_ref(&self) -> Pair<&T>;
    /// Exchanges left and right.
    pub fn swapped(self) -> Pair<T>;
}
```

#### 1.1.3 `spec`

```rust
/// Syntax of local paths. Parsing is style-explicit, so Windows rules are unit-tested on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathStyle {
    /// `/`-separated, `~` expansion, `..` kept (D-06).
    Unix,
    /// Drive letters, UNC, `\\?\` verbatim, `..` resolved lexically.
    Windows,
}

impl PathStyle {
    /// Style of the compilation target.
    pub const HOST: PathStyle = if cfg!(windows) { PathStyle::Windows } else { PathStyle::Unix };
}

/// Ambient facts needed to resolve user-typed specs. Captured by adapters; core never reads them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecContext {
    /// Local path syntax.
    pub style: PathStyle,
    /// Absolute current directory for relative paths; `None` makes relative paths an error.
    pub cwd: Option<PathBuf>,
    /// Home directory for `~`; `None` makes `~` an error.
    pub home: Option<PathBuf>,
}

impl SpecContext {
    /// A context that accepts only absolute specs (used when deserialising persisted specs).
    pub const fn absolute_only(style: PathStyle) -> SpecContext;
}

/// An absolute, normalised local path (invariants INV-SPEC-1..3, §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    /// Validates `path` (absolute and already normalised for `PathStyle::HOST`).
    pub fn new(path: PathBuf) -> Result<LocalPath, SpecError>;
    /// The path.
    pub fn as_path(&self) -> &Path;
    /// Consumes `self`.
    pub fn into_path_buf(self) -> PathBuf;
    /// Joins one name component (no separator, not `.`/`..`, no NUL).
    pub fn join_name(&self, name: &OsStr) -> Result<LocalPath, SpecError>;
    /// The last component (`None` for a root).
    pub fn file_name(&self) -> Option<&OsStr>;
    /// The parent directory (`None` for a root).
    pub fn parent(&self) -> Option<LocalPath>;
}

/// Pure, style-explicit local-path normalisation (§2.1). `cwd`/`home` must already be normalised.
pub fn normalize_local(
    input: &str,
    style: PathStyle,
    cwd: Option<&str>,
    home: Option<&str>,
) -> Result<String, SpecError>;

/// Remote URL schemes recognised by the parser (resolving them is unsupported in M1, D-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteScheme { Ftp, Ftps, Sftp, Dav, Davs, S3, Svn, SvnHttp, SvnHttps, SvnSsh, Mtp }

impl RemoteScheme {
    /// Canonical lowercase scheme text ("svn+https", …).
    pub const fn as_str(self) -> &'static str;
    /// Case-insensitive lookup.
    pub fn from_scheme(s: &str) -> Option<RemoteScheme>;
}

/// A credential taken from a URL. Never printed, never serialised; `Debug` shows `Secret(***)`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    /// Wraps a credential (additive API, §1.1.12).
    pub fn new(value: impl Into<String>) -> Secret;
    /// Reveals the value (only for back-ends; M1 never calls this).
    pub fn expose(&self) -> &str;
}

/// `scheme://[user[:password]@]host[:port][/path]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteUrl {
    /// Scheme.
    pub scheme: RemoteScheme,
    /// User name (percent-decoding is not applied in M1; stored verbatim).
    pub user: Option<String>,
    /// Password; omitted from every canonical form (VFS-010).
    pub password: Option<Secret>,
    /// Lower-cased host; an IPv6 literal keeps its brackets. For `s3://` the bucket.
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Verbatim remainder starting at the first '/' after the authority (may be empty).
    pub path: String,
}

/// An `http(s)://[user[:password]@]host[:port][rest]` single-file URL (P1 back-end). The user information is
/// kept exactly like `RemoteUrl`'s: the user verbatim, the password as a `Secret` that never appears.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HttpUrl {
    /// `https` when true.
    pub secure: bool,
    /// User name (percent-decoding is not applied in M1; stored verbatim).
    pub user: Option<String>,
    /// Password; omitted from every canonical form (VFS-010).
    pub password: Option<Secret>,
    /// Lower-cased host.
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Verbatim path, query and fragment.
    pub rest: String,
}

/// Identifies a file or folder on any back-end (ARCH 3.1). `Display` is the canonical string (ARCH-020).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Spec {
    /// Local file system path.
    Local(LocalPath),
    /// Archive member (reserved: never produced by `parse` in M1; the M2 VFS resolver creates it).
    ArchiveMember { archive: Box<Spec>, member: String },
    /// Snapshot member (reserved, P1).
    Snapshot { file: LocalPath, member: String },
    /// Remote URL (parsed, unsupported in M1).
    Remote(RemoteUrl),
    /// `profile:Name[?path]` (parsed, unsupported in M1).
    Profile { name: String, path: Option<String> },
    /// `http(s)://…` single file (parsed, unsupported in M1).
    Http(HttpUrl),
    /// `clipboard://`.
    Clipboard,
    /// `-` (standard input).
    Stdin,
}

impl Spec {
    /// Parses a user-typed spec (§2.1).
    pub fn parse(input: &str, ctx: &SpecContext) -> Result<Spec, SpecError>;
    /// Parses a command-line argument; non-UTF-8 input is accepted for local paths only.
    pub fn parse_os(input: &OsStr, ctx: &SpecContext) -> Result<Spec, SpecError>;
    /// Canonical string (same as `Display`); never contains a password.
    pub fn canonical(&self) -> String;
    /// True when `canonical()` lost information (non-UTF-8 local path). Lossy specs must not be persisted.
    pub fn is_lossy(&self) -> bool;
    /// Stable scheme name: "local", "archive", "snapshot", "ftp", …, "profile", "http", "https",
    /// "clipboard", "stdin".
    pub fn scheme_name(&self) -> &'static str;
    /// Final name component for format detection ("-" → None, Clipboard → None).
    pub fn file_name(&self) -> Option<String>;
    /// Returns a copy with every secret removed (`RemoteUrl::password = None`, `HttpUrl::password = None`).
    pub fn without_secrets(&self) -> Spec;
}

impl fmt::Display for Spec { /* canonical form, §2.1 */ }
impl Serialize for Spec { /* canonical string; a lossy spec (`is_lossy()`) is a serialisation error, so it
                              can never be persisted */ }
impl<'de> Deserialize<'de> for Spec { /* Spec::parse with SpecContext::absolute_only(PathStyle::HOST) */ }

/// Spec parsing errors (all carry the offending input; messages are stable, see §6.2.1).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SpecError {
    /// Empty or whitespace-only input.
    #[error("empty spec")]
    Empty,
    /// The input contains a NUL character.
    #[error("spec contains a NUL character")]
    Nul,
    /// Relative path without a known current directory.
    #[error("relative path {path:?} needs a current directory")]
    RelativeWithoutCwd { path: String },
    /// `~` without a known home directory.
    #[error("'~' needs a home directory")]
    HomeUnknown,
    /// `~user/…` is not supported.
    #[error("'~user' expansion is not supported: {input:?}")]
    TildeUser { input: String },
    /// `scheme://` with an unknown scheme (D-05).
    #[error("unknown URL scheme {scheme:?}")]
    UnknownScheme { scheme: String },
    /// Malformed URL authority or port.
    #[error("malformed URL {input:?}: {reason}")]
    MalformedUrl { input: String, reason: &'static str },
    /// `profile:` without a name.
    #[error("malformed profile spec {input:?}")]
    MalformedProfile { input: String },
    /// Windows `C:foo` with a current directory on another drive (or none).
    #[error("drive-relative path {path:?} cannot be resolved")]
    DriveRelative { path: String },
    /// Windows device namespace (`\\.\…`).
    #[error("device path {path:?} is not supported")]
    DevicePath { path: String },
    /// UNC path without a share (`\\server`).
    #[error("UNC path {path:?} lacks a share name")]
    UncWithoutShare { path: String },
    /// A component that `LocalPath::join_name` rejects.
    #[error("invalid file name component {name:?}")]
    InvalidComponent { name: String },
    /// `LocalPath::new` on a relative or non-normalised path.
    #[error("not an absolute normalised path: {path:?}")]
    NotNormalized { path: String },
}
```

#### 1.1.4 `path` and `case`

```rust
/// A path relative to a base folder: components joined by '/', none empty, "." or "..";
/// the empty path is the base folder itself (INV-RELPATH-1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

impl RelPath {
    /// The base folder (empty path).
    pub const fn root() -> RelPath;
    /// Parses a '/'-separated relative path.
    pub fn parse(s: &str) -> Result<RelPath, RelPathError>;
    /// Appends one name (must not contain '/' or NUL and must not be "." or "..").
    pub fn join(&self, name: &str) -> Result<RelPath, RelPathError>;
    /// Components from the base downwards.
    pub fn components(&self) -> impl Iterator<Item = &str> + '_;
    /// Last component.
    pub fn name(&self) -> Option<&str>;
    /// Parent (`None` for the root).
    pub fn parent(&self) -> Option<RelPath>;
    /// Number of components (root = 0).
    pub fn depth(&self) -> usize;
    /// The '/'-joined string.
    pub fn as_str(&self) -> &str;
    /// True for the base folder.
    pub fn is_root(&self) -> bool;
}

/// RelPath construction errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid relative path {input:?}: {reason}")]
pub struct RelPathError { pub input: String, pub reason: &'static str }

// ---- case.rs: the ONLY case and normalisation rules used anywhere in Hikasa (masks, alignment, keys) ----

/// Simple 1:1 lower-case mapping: `c.to_lowercase()` when that yields exactly one char, otherwise `c`.
/// No locale tailoring. (Example: 'İ' maps to itself because its full lowercase is two chars.)
pub fn simple_lower(c: char) -> char;
/// Simple 1:1 upper-case mapping (same rule as `simple_lower`); used for set ranges in masks (§2.2.2).
pub fn simple_upper(c: char) -> char;
/// `simple_lower` applied to every char; borrows when nothing changes.
pub fn fold(s: &str) -> Cow<'_, str>;
/// `fold(a) == fold(b)` without allocating.
pub fn eq_folded(a: &str, b: &str) -> bool;
/// Unicode NFC (`unicode-normalization`); borrows when `s` is already NFC (ASCII fast path).
pub fn nfc(s: &str) -> Cow<'_, str>;
```

#### 1.1.5 `mask` — the file mask engine (FC-050, FC-051; algorithm §2.2)

```rust
/// Case sensitivity of name matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseSensitivity { Sensitive, Insensitive }

impl CaseSensitivity {
    /// Platform default for local names: Insensitive on Windows and macOS, Sensitive elsewhere (FC-010).
    pub const HOST_DEFAULT: CaseSensitivity;
}

/// Options shared by every mask in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaskOptions {
    /// Case sensitivity (D-07: simple 1:1 folding).
    pub case: CaseSensitivity,
}

/// What a mask item applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskTarget {
    /// No trailing separator.
    Files,
    /// Trailing `\` or `/`.
    Folders,
}

/// Include (`*.c`) or exclude (`-*.bak`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity { Include, Exclude }

/// One compiled mask item.
#[derive(Debug, Clone)]
pub struct Mask {
    source: String,       // the item exactly as typed (after trimming), for round-tripping
    target: MaskTarget,
    polarity: Polarity,
    anchored: bool,       // true for `p\f`, `.\f`, `\f`; false for `f` and `...\f`
    components: Vec<ComponentPattern>, // ComponentPattern::AnyDepth for `...`
    opts: MaskOptions,
}

impl Mask {
    /// Parses one item (without ';'). Leading/trailing ASCII space and tab are trimmed.
    pub fn parse(item: &str, opts: MaskOptions) -> Result<Mask, MaskError>;
    /// The item as typed.
    pub fn source(&self) -> &str;
    /// Files or folders.
    pub fn target(&self) -> MaskTarget;
    /// Include or exclude.
    pub fn polarity(&self) -> Polarity;
    /// True when the mask has no separators (matches a name at any depth).
    pub fn is_simple(&self) -> bool;
    /// Matches a bare name (only meaningful for simple masks; relative masks return false).
    pub fn matches_name(&self, name: &str) -> bool;
    /// Full semantics (§2.2.3) against the item `parent/name`, where `parent` is the item's folder relative to
    /// the base (root = the base folder). Ignores `target`. Allocation-free for ASCII and NFC names: components
    /// are compared in place, folding is applied per char, and `case::nfc` borrows (bench `mask_match`, §6.4).
    pub fn matches(&self, parent: &RelPath, name: &str) -> bool;
    /// Convenience: `matches(path.parent(), path.name())`; the root never matches.
    pub fn matches_path(&self, path: &RelPath) -> bool;
    /// True when the folder `parent/name` is a proper prefix of some path this (anchored) mask could match
    /// (D-09 traversal). Allocation-free like `matches`.
    pub fn may_match_below(&self, parent: &RelPath, name: &str) -> bool;
    /// Sound containment test used for format shadowing: `true` only if every name matched by
    /// `self` is also matched by `other` (§2.12.3). May return false negatives, never false positives.
    pub fn is_covered_by(&self, other: &Mask) -> bool;
}

/// A `;`-separated mask list (empty items are ignored).
#[derive(Debug, Clone)]
pub struct MaskList { masks: Vec<Mask>, opts: MaskOptions }

impl MaskList {
    /// Parses a list.
    pub fn parse(list: &str, opts: MaskOptions) -> Result<MaskList, MaskError>;
    /// No items.
    pub fn is_empty(&self) -> bool;
    /// Items in order.
    pub fn masks(&self) -> &[Mask];
    /// True if any *include* item matches `name` (used for file formats, which ignore `-` items: D-10).
    pub fn matches_name(&self, name: &str) -> bool;
    /// Canonical `;`-joined text (round-trips through `parse`, INV-MASK-2).
    pub fn to_list_string(&self) -> String;
}

/// Mask syntax errors. `mask` is the item after trimming and NFC normalisation; the position `at` counts chars of
/// that NFC item (which looks the same as the item as typed).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MaskError {
    /// `[` without a matching `]`.
    #[error("mask {mask:?}: unclosed '[' at {at}")]
    UnclosedBracket { mask: String, at: usize },
    /// `[z-a]`.
    #[error("mask {mask:?}: invalid range {lo:?}-{hi:?} at {at}")]
    InvalidRange { mask: String, at: usize, lo: char, hi: char },
    /// Empty path component (`a\\b`), or a mask that is only `-`.
    #[error("mask {mask:?}: empty component")]
    EmptyComponent { mask: String },
    /// `.` used anywhere except as the first component; `..` anywhere.
    #[error("mask {mask:?}: '.' or '..' component is only allowed as a leading '.'")]
    DotComponent { mask: String },
}
```

#### 1.1.6 `filter` — NameFilter (FC-050, FC-051, FC-054; algorithm §2.3)

```rust
/// The four persisted lists of the Name Filters tab (FC-050). Order is preserved.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NameFilterLists {
    /// Include-file masks (empty = all files).
    pub include_files: Vec<String>,
    /// Exclude-file masks.
    pub exclude_files: Vec<String>,
    /// Include-folder masks (empty = all folders).
    pub include_folders: Vec<String>,
    /// Exclude-folder masks.
    pub exclude_folders: Vec<String>,
}

/// Verdict for a folder, computed top-down per side (the parent's verdict is an input; the base is `Base`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderVerdict {
    /// The base folder itself: its files are subject to file masks; it does not propagate subtree inclusion.
    Base,
    /// Scanned; its files are subject to file masks; its subfolders inherit inclusion (D-09).
    Included,
    /// Scanned only because it is a prefix of an anchored include-folder mask; its files are excluded.
    PassThrough,
    /// Not scanned; the whole subtree is excluded.
    Excluded,
}

/// Compiled name filter. Pure, `Send + Sync`, cheap to clone (Arc inside).
#[derive(Debug, Clone)]
pub struct NameFilter { /* four MaskLists + options */ }

impl NameFilter {
    /// Matches everything.
    pub fn match_all(opts: MaskOptions) -> NameFilter;
    /// Parses the toolbar syntax "*.c;*.h;-*.bak;-obj\;src\" (FC-050).
    pub fn parse(filter: &str, opts: MaskOptions) -> Result<NameFilter, MaskError>;
    /// From the four lists.
    pub fn from_lists(lists: &NameFilterLists, opts: MaskOptions) -> Result<NameFilter, MaskError>;
    /// Back to the four lists.
    pub fn to_lists(&self) -> NameFilterLists;
    /// Canonical toolbar string: include files, include folders, exclude files, exclude folders.
    pub fn to_filter_string(&self) -> String;
    /// Verdict for the folder named `name` inside `parent_path`, given the parent folder's verdict (§2.3).
    /// Allocation-free; called once per listed folder entry and side.
    pub fn folder(&self, parent_path: &RelPath, name: &str, parent: FolderVerdict) -> FolderVerdict;
    /// Whether the file named `name` inside `parent_path` is included, given the parent folder's verdict
    /// (§2.3). Allocation-free; called once per listed file entry and side.
    pub fn file(&self, parent_path: &RelPath, name: &str, parent: FolderVerdict) -> bool;
    /// True when the filter cannot exclude anything (fast path).
    pub fn is_match_all(&self) -> bool;
    /// Options used.
    pub fn options(&self) -> MaskOptions;
}
```

#### 1.1.7 `content` — the injected content-comparison seam (FC-025, FC-029)

```rust
/// Content test method (FC-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentMethod { None, Crc, Binary, Rules }

/// Result of a content comparison (FC-032 icon set). Ordered from "most same" to "most different".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ContentResult {
    /// Byte-identical (or CRC-equal for the CRC method).
    BinarySame,
    /// Bytes differ, but nothing differs once rules apply (encoding, line endings, …).
    RulesSame,
    /// Only unimportant differences ("similar").
    UnimportantOnly,
    /// At least one important difference.
    ImportantDifferences,
    /// Bytes differ (binary or CRC method, or binary fallback).
    BinaryDifferent,
    /// The comparison could not complete.
    Error { error: ErrorReport },
}

impl ContentResult {
    /// True for `BinarySame`/`RulesSame`, and for `UnimportantOnly` when `ignore_unimportant`.
    pub fn is_same(&self, ignore_unimportant: bool) -> bool;
    /// The payload-free kind (O(1); used by compact models and the IPC `ViewResult`).
    pub fn kind(&self) -> ContentKind;
}

/// `ContentResult` without the error payload. `Copy`; same variant names and serde names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind { BinarySame, RulesSame, UnimportantOnly, ImportantDifferences, BinaryDifferent, Error }

impl ContentKind {
    /// Stable snake_case name ("unimportant_only", …), identical to the serde form.
    pub const fn as_str(self) -> &'static str;
    /// Inverse of `as_str` (`None` for unknown names, e.g. from a newer peer).
    pub fn parse(s: &str) -> Option<ContentKind>;
}

/// A content result plus the method that produced it (CRC-equal is not proof of identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentOutcome {
    /// Method requested.
    pub method: ContentMethod,
    /// Result.
    pub result: ContentResult,
    /// Non-fatal notes (e.g. `fell_back_to_binary`).
    pub warnings: Vec<Warning>,
}

/// Read access to one file's content, independent of any VFS (implemented by hikasa-vfs, and by tests).
pub trait ContentSource: Send + Sync {
    /// File name used for format detection (no directories).
    fn name(&self) -> &str;
    /// Canonical spec string for messages.
    fn display(&self) -> String;
    /// Length in bytes when known.
    fn len(&self) -> Option<u64>;
    /// CRC32 known without reading (archives, snapshots).
    fn known_crc32(&self) -> Option<u32> { None }
    /// Local path when the content is a local regular file (enables mmap in hikasa-vfs).
    fn local_path(&self) -> Option<&Path> { None }
    /// Opens a fresh sequential reader at offset 0.
    fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>>;
    /// Reads the whole content into an exact-size buffer. Checks `cancel` at least every 1 MiB read.
    fn read_all(&self, limit: u64, cancel: &CancelToken) -> Result<Vec<u8>, ReadError>;
}

/// Failure of `ContentSource::read_all`. `Classify`: TooLarge → `too_large`, Io → `io_error_kind`,
/// Cancelled → `cancelled`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The content is larger than the caller's limit.
    #[error("content larger than {limit} bytes")]
    TooLarge { limit: u64 },
    /// I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// `cancel` fired.
    #[error("operation cancelled")]
    Cancelled,
}

/// A content test. Implementations: hikasa-vfs `CrcComparator`, `BinaryComparator`;
/// hikasa-text `RulesComparator` (injected into hikasa-folder, which never depends on hikasa-text).
pub trait ContentComparator: Send + Sync {
    /// The method implemented.
    fn method(&self) -> ContentMethod;
    /// Largest file (bytes) this comparator loads completely; larger files are handled by a documented
    /// fallback (`RulesComparator`: byte comparison, D-25). `None` = streams, no limit.
    fn size_limit(&self) -> Option<u64> { None }
    /// Compares two sources. Cancellation is not a result: it returns `Err(Cancelled)`.
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled>;
}
```

#### 1.1.8 `cancel`

```rust
/// Cooperative cancellation. Cloning shares the flag; `child()` creates a token that is cancelled when its
/// parent is (INV-CANCEL-1), while cancelling a child never cancels the parent (INV-CANCEL-2).
#[derive(Debug, Clone)]
pub struct CancelToken { inner: Option<Arc<CancelInner>> } // None = never cancelled (const-constructible)

impl CancelToken {
    /// A new, live token.
    pub fn new() -> CancelToken;
    /// A token that is never cancelled (tests, quick helpers).
    pub const fn never() -> CancelToken;
    /// Requests cancellation (idempotent; Release ordering).
    pub fn cancel(&self);
    /// True if this token or any ancestor was cancelled (Acquire ordering).
    pub fn is_cancelled(&self) -> bool;
    /// `Err(Cancelled)` when cancelled, for `?` use at check points.
    pub fn check(&self) -> Result<(), Cancelled>;
    /// A child token.
    pub fn child(&self) -> CancelToken;
}

/// Marker error: the operation observed cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("operation cancelled")]
pub struct Cancelled;
```

#### 1.1.9 `progress`, `job`

```rust
/// What a job is doing (for status bars and stderr progress).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Phase { Reading, Decoding, Tokenizing, Aligning, Classifying, Scanning, ComparingContent, Hashing, Reporting }

/// Unit of `done`/`total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit { Items, Bytes, Lines, Folders }

/// A progress report. `total == None` means indeterminate. `done` never decreases within a phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// Phase.
    pub phase: Phase,
    /// Units done.
    pub done: u64,
    /// Units total, when known.
    pub total: Option<u64>,
    /// Unit.
    pub unit: Unit,
    /// Item being processed (e.g. a relative path).
    pub current: Option<String>,
}

/// Receives progress. Must be cheap and non-blocking; it is called from worker threads.
pub trait ProgressSink: Send + Sync {
    /// Receives one report.
    fn report(&self, progress: Progress);
    /// Whether a report for `phase` would be forwarded now (the throttle is open). Engines call it through
    /// `Ctl::progress` so that a report is built only when it will be used. Default: always.
    fn due(&self, _phase: Phase) -> bool { true }
}

impl<F: Fn(Progress) + Send + Sync> ProgressSink for F { /* calls the closure */ }

/// Discards everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullProgress;

/// Forwards at most one report per `interval` per phase. Each phase has its own throttle, so phases that report
/// concurrently (folder compare: `scanning` and `comparing_content`) never open each other's throttle. A report
/// of a phase is due when nothing of that phase was forwarded yet or `interval` has elapsed since the last
/// forwarded report of that phase; a completion (`done == total`) is always forwarded. Uses `Instant::now()`,
/// which affects only progress delivery, never results (allowed by T1).
pub struct ThrottledSink<S: ProgressSink> { inner: S, interval: Duration, state: Mutex<ThrottleState> }

impl<S: ProgressSink> ThrottledSink<S> {
    /// Wraps `inner`.
    pub fn new(inner: S, interval: Duration) -> Self;
}
impl<S: ProgressSink> ProgressSink for ThrottledSink<S> {
    /* report: forwards when the report completes its phase (`done == total`) or `due(phase)`, and then records
       the time for that phase. due: true when nothing of `phase` was forwarded yet or `interval` has elapsed
       since the last forwarded report of `phase` (per phase; other phases do not matter). */
}

/// Cancellation and progress for one long operation.
#[derive(Clone, Copy)]
pub struct Ctl<'a> {
    /// Cancellation token.
    pub cancel: &'a CancelToken,
    /// Progress sink.
    pub progress: &'a dyn ProgressSink,
}

impl Ctl<'static> {
    /// Never cancelled, discards progress (tests and synchronous helpers).
    pub fn detached() -> Ctl<'static>;
}

impl<'a> Ctl<'a> {
    /// `self.cancel.check()`.
    pub fn check(&self) -> Result<(), Cancelled>;
    /// Lazy, throttled report: calls `make` (and forwards its result) only when `self.progress.due(phase)`.
    /// Hot loops use this, so no `Progress` (and no `current` string) is built between throttle ticks.
    pub fn progress(&self, phase: Phase, make: impl FnOnce() -> Progress);
    /// Unconditional report (phase starts and completion `done == total`).
    pub fn progress_now(&self, progress: Progress);
}

/// Identifier of a job; unique within one `JobIdAllocator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

/// Kind of job (status bar text, log grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobKind { QuickCompare, TextCompare, FolderCompare, Report, FormatTokens, VfsList, VfsRead }

/// Allocates increasing ids starting at 1. Owned by a service instance (never global, T7).
#[derive(Debug, Default)]
pub struct JobIdAllocator(AtomicU64);

impl JobIdAllocator {
    /// New allocator.
    pub const fn new() -> Self;
    /// Next id (Relaxed ordering; uniqueness only).
    pub fn next(&self) -> JobId;
}
```

#### 1.1.10 `time`

```rust
/// A UTC instant with nanosecond precision (INV-TIME-1: `nanos < 1_000_000_000`).
/// Serialises as RFC 3339 UTC via jiff (`2026-10-19T13:12:00.5Z`; fractional digits only when non-zero).
/// Instants outside jiff's range serialise as `"@<secs>.<9-digit nanos>"`, and that form parses back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileTime { secs: i64, nanos: u32 }

impl FileTime {
    /// From Unix seconds and nanoseconds (`None` if `nanos >= 1e9`).
    pub const fn from_unix(secs: i64, nanos: u32) -> Option<FileTime>;
    /// From `SystemTime` (handles instants before the epoch).
    pub fn from_system_time(t: SystemTime) -> FileTime;
    /// Unix seconds (floor).
    pub const fn unix_seconds(self) -> i64;
    /// Sub-second nanoseconds.
    pub const fn subsec_nanos(self) -> u32;
    /// `other - self` in nanoseconds (exact; i128 cannot overflow).
    pub const fn nanos_until(self, other: FileTime) -> i128;
    /// RFC 3339 text (same as `Display`).
    pub fn to_rfc3339(self) -> String;
    /// Parses RFC 3339 (any offset, normalised to UTC) or the `@secs.nanos` form.
    pub fn parse(s: &str) -> Result<FileTime, TimeParseError>;
}

impl fmt::Display for FileTime { /* RFC 3339 */ }
impl Serialize for FileTime { /* string */ }
impl<'de> Deserialize<'de> for FileTime { /* FileTime::parse */ }

/// `FileTime::parse` failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid timestamp {input:?}: {reason}")]
pub struct TimeParseError {
    /// The rejected text.
    pub input: String,
    /// Stable reason ("not RFC 3339", "nanoseconds out of range", …).
    pub reason: &'static str,
}
```

#### 1.1.11 `exit`, `error`, `warning`, `schema`

```rust
/// Process exit codes (CLI-004), shared by every subcommand. Mapping rules: §2.21.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    BinarySame = 1,
    RulesSame = 2,
    BinaryDifferent = 11,
    Similar = 12,
    RulesDifferent = 13,
    Conflicts = 14,
    UnknownError = 100,
    ConflictsNotWritten = 101,
    CannotWaitForGui = 102,
    GuiNotFound = 103,
    ScriptLoadError = 105,
    ScriptSyntaxError = 106,
    LoadFailed = 107,
}

impl ExitCode {
    /// Numeric value.
    pub const fn code(self) -> u8;
    /// Inverse of `code`.
    pub const fn from_code(code: u8) -> Option<ExitCode>;
    /// Verdict mapping for `quick`, `text compare` and GUI file views (§2.21, D-01).
    pub fn for_content(result: &ContentResult, ignore_unimportant: bool) -> ExitCode;
    /// Same mapping on the payload-free kind (`for_content(r, iu) == for_kind(r.kind(), iu)`).
    pub fn for_kind(kind: ContentKind, ignore_unimportant: bool) -> ExitCode;
    /// Verdict mapping for `folder compare` (§2.21, D-02).
    pub fn for_verdict(verdict: CompareVerdict) -> ExitCode;
}

/// Overall verdict of a multi-item comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareVerdict { NoDifferences, UnimportantOnly, Differences, Incomplete }

impl CompareVerdict {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str;
    /// Inverse of `as_str` (`None` for unknown names).
    pub fn parse(s: &str) -> Option<CompareVerdict>;
}

/// Coarse classification of every error in Hikasa (stable JSON `error.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorKind {
    NotFound, PermissionDenied, AlreadyExists, NotADirectory, IsADirectory, InvalidInput,
    InvalidData, Unsupported, Cancelled, Io, TooLarge, Protocol, Internal,
}

/// Implemented by every crate error.
pub trait Classify {
    /// The error's kind.
    fn kind(&self) -> ErrorKind;
}

/// Maps `std::io::ErrorKind` to `ErrorKind` (total; unknown kinds → `Io`).
pub fn io_error_kind(e: &std::io::Error) -> ErrorKind;

/// Serialisable snapshot of an error (JSON output, IPC, per-node error states).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReport {
    /// Kind.
    pub kind: ErrorKind,
    /// Top-level message (`Display` of the error).
    pub message: String,
    /// Canonical spec involved, if any.
    pub spec: Option<String>,
    /// `source()` chain messages, outermost first.
    pub causes: Vec<String>,
}

impl ErrorReport {
    /// Captures `err` (walks `source()`).
    pub fn capture<E: std::error::Error + Classify + ?Sized>(err: &E, spec: Option<&Spec>) -> ErrorReport;
}

/// An invariant check failed (always a bug; reported as `ErrorKind::Internal`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invariant {id} violated: {detail}")]
pub struct InvariantViolation {
    /// Invariant id from Appendix B (e.g. "INV-ALIGN-1").
    pub id: &'static str,
    /// Human-readable detail.
    pub detail: String,
}

/// A non-fatal observation returned inside results (T8).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Warning {
    /// Stable code.
    pub code: WarningCode,
    /// Message.
    pub message: String,
    /// Spec concerned, if any.
    pub spec: Option<String>,
}

/// Stable warning codes (JSON `warnings[].code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WarningCode {
    BinaryContent, LossyDecode, RegexRuntimeError, FellBackToBinary, RulesSizeLimit, ClosenessIgnored,
    WorkBudgetExhausted, ClosenessBudgetExhausted, FormatMismatch, CapabilityMissing, UnknownConfigKey,
    UserFormatSkipped, PatchLossyEol, SymlinkLoop, EntryError, SwitchIgnored,
    AlignmentDegraded,   // wave 2: hikasa-text maps hikasa_diff::Alignment::diff_warnings() to it (§1.3.3)
    PatchEncoding,       // wave 4: a patch written in UTF-8 for a file stored in another encoding (§1.7)
}

/// Outcome of a schema check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaCompat {
    /// Same version.
    Current,
    /// Older file; migrate in memory (SES-052).
    Older { found: u32 },
}

/// Newer-than-supported file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("schema_version {found} is newer than the supported version {supported}")]
pub struct SchemaError { pub found: u32, pub supported: u32 }

/// SES-052 check used by every TOML loader.
pub const fn check_schema_version(found: u32, supported: u32) -> Result<SchemaCompat, SchemaError>;
```

#### 1.1.12 Accepted additive API (wave 1)

Wave 1 added the following public items. They are part of the contract: later waves may use them, and they
change no signature above.

```rust
impl Secret { pub fn new(value: impl Into<String>) -> Secret; }          // wraps a credential
impl RemoteScheme { pub const ALL: [RemoteScheme; 11]; }                // declaration order
impl ContentKind  { pub const ALL: [ContentKind; 6]; }                  // declaration order
impl ExitCode     { pub const ALL: [ExitCode; 14]; }                    // numeric order

/// The `TryFrom<u8>` error of `ExitCode` (serde `try_from = "u8"`). `Classify`: `invalid_data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("unknown exit code {0}")]
pub struct UnknownExitCode(pub u8);

impl MaskList { pub fn options(&self) -> MaskOptions; }                 // the options of every item

// Stable snake_case names, identical to the serde forms:
impl ContentMethod { pub const fn as_str(self) -> &'static str; }
impl ErrorKind     { pub const fn as_str(self) -> &'static str; }
impl WarningCode   { pub const fn as_str(self) -> &'static str; }
impl Phase         { pub const fn as_str(self) -> &'static str; }
impl JobKind       { pub const fn as_str(self) -> &'static str; }
// Display: ContentKind, CompareVerdict, ErrorKind, WarningCode (as_str); ExitCode and JobId (the number);
// RelPath (the '/'-joined string); ErrorReport (the message); RemoteUrl and HttpUrl (canonical, no password).

impl<S: ProgressSink + ?Sized> ProgressSink for Arc<S> { /* forwards report and due */ }
impl From<Cancelled> for ReadError { /* ReadError::Cancelled */ }
// Classify: RelPathError, TimeParseError (invalid_input); SchemaError (unsupported); UnknownExitCode
// (invalid_data); std::io::Error (io_error_kind); Cancelled (cancelled); ErrorReport (its kind);
// InvariantViolation (internal). io_error_kind maps an io::Error that wraps `Cancelled` to `cancelled`.
```

`MaskError` positions (`at`) count chars of the item after trimming and NFC normalisation (§1.1.5).

### 1.2 hikasa-vfs

**Purpose.** Every file system access in M1 (VFS-001). Back-ends: `LocalVfs`; `MemoryVfs`, which tests use
as a deterministic fake and which supports fault injection; and the stdin and clipboard blobs injected by
adapters. Content helpers: streaming CRC32, binary compare, and an mmap fast path.
**Uses.** hikasa-core, serde, thiserror, crc32fast, memmap2, filetime, bitflags, same-file.

#### 1.2.1 Module tree

```
core/hikasa-vfs/src/
├── lib.rs          #![deny(unsafe_code)]; `#[allow(unsafe_code)] mod mmap;` is the only exemption
├── entry.rs        Entry, EntryKind, FileName, Attrs, DosAttrs, UnixMeta, FileId
├── caps.rs         Capabilities
├── vfs.rs          trait Vfs, ReadSeek, AtomicWrite, FollowLinks, ListOptions, Prefilter, Listing, NameOnly,
│                   EntryError, WriteOptions
├── error.rs        VfsError, VfsOp
├── local/
│   ├── mod.rs      LocalVfs, LocalOptions
│   ├── unix.rs     cfg(unix): mode/uid/gid/dev/ino mapping via std::os::unix::fs::MetadataExt
│   └── windows.rs  cfg(windows): DosAttrs via std::os::windows::fs::MetadataExt::file_attributes
├── memory.rs       MemoryVfs, MemMeta, ListingOrder, Fault
├── router.rs       VfsRouter, Blob
├── source.rs       VfsSource (impl hikasa_core::ContentSource)
├── stream.rs       StreamOptions, MmapPolicy, crc32_reader, compare_readers
├── compare.rs      BinaryOutcome, crc32_source, compare_sources, CrcComparator, BinaryComparator
└── mmap.rs         ONLY `unsafe` in library and program code (§6.1 exempts test/bench allocation counters):
                    mapped compare and mapped CRC with re-stat; slices never escape
```

#### 1.2.2 Entry model

```rust
/// Entry kind. `Ord` puts folders first (used for deterministic child ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind { Dir, File, Symlink, Other }

/// A file name. `as_str()` is the UTF-8 form, lossy when needed, used for matching and display.
/// `raw` keeps the exact OS name when that conversion was lossy (INV-NAME-1: raw.is_some() ⇔ lossy).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileName { text: Box<str>, raw: Option<OsString> }

impl FileName {
    /// From an OS name (never fails; lossy names keep `raw`).
    pub fn from_os(name: &OsStr) -> FileName;
    /// From UTF-8; rejects "", ".", "..", '/', NUL (and '\\' on Windows).
    pub fn new(name: &str) -> Result<FileName, VfsError>;
    /// UTF-8 (possibly lossy) text.
    pub fn as_str(&self) -> &str;
    /// Exact OS name.
    pub fn to_os_string(&self) -> OsString;
    /// True when `as_str()` is lossy.
    pub fn is_lossy(&self) -> bool;
    /// Text after the last '.', when that '.' is neither the first nor the last char (".bashrc" → None).
    pub fn extension(&self) -> Option<&str>;
    /// Name without `.extension` (the whole name when there is no extension).
    pub fn stem(&self) -> &str;
}

bitflags::bitflags! {
    /// DOS/Windows attributes; values equal the Win32 FILE_ATTRIBUTE_* constants. Letters per D-30.
    /// No serde derive: the hand-written impls below serialise the canonical letter string (D-30).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct DosAttrs: u32 {
        /// R
        const READONLY = 0x0000_0001;
        /// H
        const HIDDEN = 0x0000_0002;
        /// S
        const SYSTEM = 0x0000_0004;
        /// D
        const DIRECTORY = 0x0000_0010;
        /// A
        const ARCHIVE = 0x0000_0020;
        /// N
        const NORMAL = 0x0000_0080;
        /// T
        const TEMPORARY = 0x0000_0100;
        /// Z
        const SPARSE = 0x0000_0200;
        /// L
        const REPARSE_POINT = 0x0000_0400;
        /// C
        const COMPRESSED = 0x0000_0800;
        /// O
        const OFFLINE = 0x0000_1000;
        /// I
        const NOT_CONTENT_INDEXED = 0x0000_2000;
        /// E
        const ENCRYPTED = 0x0000_4000;
        /// P
        const PINNED = 0x0008_0000;
        /// U
        const UNPINNED = 0x0010_0000;
        /// M
        const RECALL_ON_DATA_ACCESS = 0x0040_0000;
    }
}

impl DosAttrs {
    /// Letters in canonical order `A C D E H I L M N O P R S T U Z` (FC-040).
    pub fn letters(self) -> String;
    /// Parses letters (case-insensitive, any order, duplicates allowed); unknown letters are an error.
    pub fn parse_letters(s: &str) -> Result<DosAttrs, VfsError>;
}
impl Serialize for DosAttrs { /* the string `self.letters()`, e.g. "HRS"; empty set = "" */ }
impl<'de> Deserialize<'de> for DosAttrs { /* `DosAttrs::parse_letters`; errors become serde errors */ }

/// Unix metadata (FC-024). `mode` holds permission bits only (`st_mode & 0o7777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnixMeta {
    /// Permission bits including setuid/setgid/sticky.
    pub mode: u32,
    /// Owner uid (names are not resolved in M1: std has no API for it).
    pub uid: u32,
    /// Group gid.
    pub gid: u32,
}

impl UnixMeta {
    /// `drwxr-xr-x` style string (first char from `kind`; s/S/t/T for special bits).
    pub fn permissions_string(&self, kind: EntryKind) -> String;
}

/// Platform attributes of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Attrs {
    /// Back-end has no attributes.
    None,
    /// Windows.
    Dos { attrs: DosAttrs },
    /// Unix.
    Unix { meta: UnixMeta },
}

/// File-object identity for loop detection (Unix device + inode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId {
    /// Device.
    pub dev: u64,
    /// Inode.
    pub ino: u64,
}

/// Metadata of one entry (ARCH 3.2). Pure data; equality is field-wise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Name within its parent (a root, and the stdin and clipboard blobs, carry their spec text, §1.2.7).
    pub name: FileName,
    /// Kind (of the target when `followed`).
    pub kind: EntryKind,
    /// Files: byte length. Folders: 0. Unfollowed symlinks: byte length of the exact target (raw OS bytes on
    /// Unix, UTF-8 on Windows; equals `link_target.len()` unless the target is lossy).
    pub size: u64,
    /// Last modification time.
    pub mtime: Option<FileTime>,
    /// Creation (birth) time where available. NOT the POSIX status-change time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Attrs,
    /// Symlink target text (for unfollowed links, and for followed ones for display); UTF-8, lossy when the
    /// target is not (see `link_target_raw`).
    pub link_target: Option<String>,
    /// The exact OS target when `link_target` is lossy (`Some` only then, like `FileName`'s raw name;
    /// INV-NAME-1). Wave 2 (§1.2.7); exact comparisons use `Entry::link_target_os()`.
    pub link_target_raw: Option<OsString>,
    /// True when this entry is a symlink that was followed (the other fields describe the target).
    pub followed: bool,
    /// CRC32 provided by the back-end, or computed earlier in this run.
    pub crc32: Option<u32>,
    /// Identity for loop detection (Unix, and `MemoryVfs` with `dev` 0 and a node id; `None` elsewhere).
    pub file_id: Option<FileId>,
}
```

#### 1.2.3 Capabilities and the `Vfs` trait

```rust
bitflags::bitflags! {
    /// What a back-end can do at a given spec (ARCH-030).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Capabilities: u32 {
        /// `open_read`.
        const READ = 1 << 0;
        /// `open_write`.
        const WRITE = 1 << 1;
        /// `list`.
        const LIST = 1 << 2;
        /// Readers implement `Seek`.
        const SEEK = 1 << 3;
        /// `Entry::mtime` is provided.
        const MTIME = 1 << 4;
        /// `Entry::ctime` is provided.
        const CTIME = 1 << 5;
        /// Timestamps can be set (not used in M1).
        const SET_TIMES = 1 << 6;
        /// `Attrs::Dos`.
        const DOS_ATTRS = 1 << 7;
        /// `Attrs::Unix` mode.
        const UNIX_MODE = 1 << 8;
        /// `Attrs::Unix` uid/gid.
        const UNIX_OWNER = 1 << 9;
        /// Symlinks are reported.
        const SYMLINKS = 1 << 10;
        /// `Entry::crc32` is provided without reading.
        const CRC_PROVIDED = 1 << 11;
        /// `local_path()` returns paths (mmap possible).
        const LOCAL_PATH = 1 << 12;
        /// Names are case-sensitive.
        const CASE_SENSITIVE = 1 << 13;
    }
}

/// Whether symlinks are followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowLinks {
    /// Report the link itself (lstat).
    #[default]
    No,
    /// Report the target (stat); broken links are reported unfollowed with an `EntryError` (in `list`: in
    /// `entries` and in `errors`, §1.2.7).
    Yes,
}

/// Decides, from the name and the kind found in the directory record, whether an entry is stat'ed
/// (`true`) or reported name-only (`false`, D-45). Called before any per-entry system call.
pub type Prefilter<'a> = &'a (dyn Fn(&str, EntryKind) -> bool + Sync);

/// Listing options.
#[derive(Clone, Copy, Default)]
pub struct ListOptions<'a> {
    /// Symlink handling.
    pub follow: FollowLinks,
    /// Optional prefilter. Back-ends call it only when the directory record states the kind (`d_type`,
    /// Windows find data); entries of unknown kind and symlinks are always stat'ed and never passed to it.
    /// `LocalVfs`: when the record states no kind, std's `DirEntry::file_type` `lstat`s the entry and the
    /// prefilter receives the kind found (accepted deviation, §1.2.7).
    pub prefilter: Option<Prefilter<'a>>,
}
impl fmt::Debug for ListOptions<'_> { /* prints `follow` and whether a prefilter is set */ }

/// An entry reported without metadata because the prefilter declined it (D-45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameOnly {
    /// The entry name.
    pub name: FileName,
    /// Kind from the directory record (`Dir`, `File` or `Other`; never `Symlink`).
    pub kind: EntryKind,
}

/// Result of `list`. Order of `entries` and `name_only` is UNSPECIFIED (consumers must sort; INV-VFS-1).
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Entries that could be described.
    pub entries: Vec<Entry>,
    /// Entries the prefilter declined (not stat'ed).
    pub name_only: Vec<NameOnly>,
    /// Names whose metadata could not be read (the listing itself succeeded). NOT disjoint from `entries`: with
    /// `FollowLinks::Yes` a broken or looping symlink is in `entries` (unfollowed) and has an error here; the
    /// consumer attaches the error to that entry instead of creating a second item (§1.2.7).
    pub errors: Vec<EntryError>,
}

/// A per-entry failure inside a successful listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryError {
    /// The entry name.
    pub name: FileName,
    /// The failure.
    pub error: ErrorReport,
}

/// Write options (minimal in M1: report output files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Create missing parent folders.
    pub create_parents: bool,
    /// Replace an existing file (otherwise `AlreadyExists`).
    pub overwrite: bool,
}

/// `Read + Seek + Send`.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// An atomic writer: nothing is visible at the target until `commit`; dropping without `commit` discards the
/// temporary file (SES-053, INV-VFS-3).
pub trait AtomicWrite: Write + Send {
    /// Flushes, syncs and atomically renames onto the target.
    fn commit(self: Box<Self>) -> Result<(), VfsError>;
}

/// A virtual file system back-end (VFS-001). Implementations are `Send + Sync` and stateless per call
/// (the CLI and GUI share one instance across threads).
pub trait Vfs: Send + Sync {
    /// Capabilities at `at` (may depend on the mount for local paths; M1 uses platform defaults).
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError>;
    /// Metadata of `spec`.
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError>;
    /// Lists a folder. Checks `cancel` at least every 1024 entries. Honours `opts.prefilter` (D-45).
    fn list(&self, dir: &Spec, opts: &ListOptions<'_>, cancel: &CancelToken) -> Result<Listing, VfsError>;
    /// Opens for streaming reads.
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError>;
    /// Opens an atomic writer.
    fn open_write(&self, spec: &Spec, opts: &WriteOptions) -> Result<Box<dyn AtomicWrite>, VfsError>;
    /// The spec of child `name` inside folder `dir`.
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError>;
    /// A local path for `spec` when it is a local regular file (enables mmap). The default body names its
    /// parameter `_spec` (§0.4 rule 6).
    fn local_path(&self, _spec: &Spec) -> Option<PathBuf> { None }
}

/// VFS operation names (error context).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfsOp { Stat, List, Open, Read, Write, Commit, Map }

/// VFS errors. `Classify` maps them to `ErrorKind` one-to-one by variant (Io via `io_error_kind`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VfsError {
    /// Missing.
    #[error("{spec}: not found")]
    NotFound { spec: String },
    /// Permission denied.
    #[error("{spec}: permission denied")]
    PermissionDenied { spec: String },
    /// Expected a folder.
    #[error("{spec}: not a folder")]
    NotADirectory { spec: String },
    /// Expected a file.
    #[error("{spec}: is a folder")]
    IsADirectory { spec: String },
    /// Write target exists and `overwrite` was false.
    #[error("{spec}: already exists")]
    AlreadyExists { spec: String },
    /// Spec kind or operation unsupported in this version (D-05).
    #[error("{spec}: {what} is not supported in this version")]
    Unsupported { spec: String, what: &'static str },
    /// Content larger than a caller-imposed limit.
    #[error("{spec}: larger than the limit of {limit} bytes")]
    TooLarge { spec: String, limit: u64 },
    /// Invalid file name.
    #[error("invalid file name {name:?}")]
    InvalidName { name: String },
    /// Other I/O failure.
    #[error("{spec}: {op:?} failed: {source}")]
    Io { spec: String, op: VfsOp, #[source] source: std::io::Error },
    /// Cancelled during a VFS call.
    #[error("operation cancelled")]
    Cancelled,
    /// DOS attribute letters outside `A C D E H I L M N O P R S T U Z` (`DosAttrs::parse_letters`). Wave 2
    /// (§1.2.7); `Classify`: `invalid_input`.
    #[error("invalid DOS attribute letters {input:?}")]
    InvalidAttributes { input: String },
}
```

#### 1.2.4 Back-ends

```rust
/// Local file system back-end.
#[derive(Debug, Clone, Default)]
pub struct LocalVfs { opts: LocalOptions }

/// Local back-end options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalOptions {
    /// Overrides the platform default case sensitivity (`CaseSensitivity::HOST_DEFAULT`).
    pub case: Option<CaseSensitivity>,
}

impl LocalVfs {
    /// New local back-end.
    pub fn new(opts: LocalOptions) -> LocalVfs;
}
impl Vfs for LocalVfs { /* Spec::Local only; other variants → Unsupported("non-local spec") */ }

/// Deterministic in-memory file system for tests (and future clipboard/stdin trees). Paths are
/// `Spec::Local` absolute paths. Thread-safe (std `RwLock` inside). `Debug` (counts, not contents) and
/// `Default` (case-sensitive) are implemented by hand (§1.2.7).
pub struct MemoryVfs { /* tree + options */ }

/// Metadata for inserted entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemMeta {
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Option<Attrs>,
    /// Back-end CRC (simulates archives).
    pub crc32: Option<u32>,
}

/// Listing order produced by `MemoryVfs::list`. It exposes bugs that depend on order (INV-VFS-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListingOrder {
    /// Byte order of names.
    #[default]
    Sorted,
    /// Reverse byte order.
    Reversed,
    /// Deterministic shuffle (xorshift64* seeded with `seed`).
    Shuffled { seed: u64 },
}

/// An injected failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Affected path.
    pub path: String,
    /// Operation that fails (`Map` has no effect: `MemoryVfs` has no local paths, so nothing is mapped).
    pub op: VfsOp,
    /// Resulting error kind (`protocol` and `internal`, which no file system error carries, give
    /// `VfsError::Io` classified `io`).
    pub kind: ErrorKind,
}

impl MemoryVfs {
    /// Empty file system with the given name case sensitivity.
    pub fn new(case: CaseSensitivity) -> MemoryVfs;
    /// Inserts a file (creating parents).
    pub fn insert_file(&self, path: &str, content: impl Into<Arc<[u8]>>, meta: MemMeta) -> Result<(), VfsError>;
    /// Inserts a folder (creating parents).
    pub fn insert_dir(&self, path: &str, meta: MemMeta) -> Result<(), VfsError>;
    /// Inserts a symlink.
    pub fn insert_symlink(&self, path: &str, target: &str, meta: MemMeta) -> Result<(), VfsError>;
    /// Sets the listing order.
    pub fn set_listing_order(&self, order: ListingOrder);
    /// Adds a fault.
    pub fn inject(&self, fault: Fault);
    /// Content of a file written through `open_write` (for asserting report outputs).
    pub fn read_back(&self, path: &str) -> Option<Arc<[u8]>>;
}
impl Vfs for MemoryVfs {
    /* full semantics incl. FollowLinks, loops (the local file system's ELOOP error) and the prefilter (every kind
       is known unless `set_kind_unknown` says otherwise, §1.2.7); `list` checks `cancel` every 1024 entries while
       filtering and while describing */
}

/// In-memory content injected by an adapter (stdin bytes read once; clipboard text). `Debug` is implemented by
/// hand: it shows the length, not the bytes (§1.2.7).
#[derive(Clone)]
pub struct Blob {
    /// Bytes.
    pub bytes: Arc<[u8]>,
    /// Timestamp to report (the adapter passes "now" at read time; VFS table: timestamps = now).
    pub mtime: FileTime,
    /// Name hint for format detection (`--stdin-name`, D-15); `None` = no name.
    pub name: Option<String>,
}


/// Dispatches specs to back-ends (table below). This is the `Vfs` the session layer uses.
#[derive(Debug, Clone)]
pub struct VfsRouter { local: LocalVfs, stdin: Option<Blob>, clipboard: Option<Blob> }

impl VfsRouter {
    /// Router over a local back-end.
    pub fn new(local: LocalVfs) -> VfsRouter;
    /// Makes `Spec::Stdin` readable.
    pub fn with_stdin(self, blob: Blob) -> VfsRouter;
    /// Makes `Spec::Clipboard` readable (no clipboard crate in M1: adapters may leave this unset).
    pub fn with_clipboard(self, blob: Blob) -> VfsRouter;
    /// The name hint of the blob behind `Spec::Stdin` / `Spec::Clipboard` (D-15); `None` otherwise.
    pub fn name_hint(&self, spec: &Spec) -> Option<&str>;
}
impl Vfs for VfsRouter { /* see dispatch table */ }
```

Router dispatch (normative):

| Spec | `stat`/`open_read` | `list` | `open_write` | `capabilities` |
|---|---|---|---|---|
| `Local` | `LocalVfs` | `LocalVfs` | `LocalVfs` (atomic temp + rename) | platform set |
| `Stdin` | blob (name `-`, kind File, mtime from blob), else `Unsupported{what:"stdin was not provided"}` | `NotADirectory` | `Unsupported{what:"writing to standard input"}` | READ \| SEEK \| MTIME |
| `Clipboard` | blob (name `clipboard://`, kind File, mtime from blob), else `Unsupported{what:"clipboard access"}` | `NotADirectory` | `Unsupported{what:"writing to the clipboard"}` | READ \| SEEK \| MTIME |
| `ArchiveMember`, `Snapshot`, `Remote`, `Profile`, `Http` | `Unsupported{what:"<scheme> specs"}` | same | same | `Unsupported` |

#### 1.2.5 Content helpers (FC-025, VFS-020, ARCH-081)

```rust
/// Streaming parameters. `Default` = 1 MiB buffers, `MmapPolicy::Auto { min_len: 16 MiB }`, one thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    /// Read buffer per side (OPT "Buffer size for binary compare"); clamped to [4 KiB, 64 MiB]. Default 1 MiB.
    pub buffer_size: usize,
    /// When to memory-map local files.
    pub mmap: MmapPolicy,
    /// Threads that process one memory-mapped file (CRC) or file pair (comparison): the calling thread plus up
    /// to `threads - 1` scoped helper threads that end before the call returns (§5.1). Clamped to [1, 16] and
    /// to the number of 8 MiB chunks. Default 1 (the engine API starts no thread unless asked);
    /// `hikasa-session` passes `min(4, its pool size)`. Results never depend on it (T2, §2.20). The streaming
    /// path always runs on the calling thread. Wave 2 (§1.2.7).
    pub threads: usize,
}

/// mmap policy (the streaming path is the reference implementation; INV-VFS-2: both give equal results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapPolicy {
    /// Never map.
    Never,
    /// Map local regular files whose length is at least `min_len` (default 16 MiB).
    Auto { min_len: u64 },
}

/// Outcome of a binary comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BinaryOutcome {
    /// Identical bytes.
    Same,
    /// Different; `first_difference` is the first differing offset, or `None` when decided by length alone.
    Different { first_difference: Option<u64> },
}

/// CRC32 (IEEE, `crc32fast`) of a reader. Progress in bytes; cancellation checked per buffer and between the
/// partial reads that fill it. Read errors name the reader `<reader>` (§1.2.7).
pub fn crc32_reader(r: &mut dyn Read, opts: &StreamOptions, ctl: &Ctl<'_>) -> Result<u32, VfsError>;
/// Byte-wise comparison of two readers (alternating fills of equal-sized chunks). Read errors name the reader
/// `<first reader>` or `<second reader>`.
pub fn compare_readers(a: &mut dyn Read, b: &mut dyn Read, opts: &StreamOptions, ctl: &Ctl<'_>)
    -> Result<BinaryOutcome, VfsError>;
/// CRC of a source: `known_crc32()` first, then mmap (policy permitting), then streaming.
pub fn crc32_source(src: &dyn ContentSource, opts: &StreamOptions, ctl: &Ctl<'_>) -> Result<u32, VfsError>;
/// Binary comparison of two sources (algorithm §2.20): same-object shortcut, length shortcut, mmap, stream.
pub fn compare_sources(a: &dyn ContentSource, b: &dyn ContentSource, opts: &StreamOptions, ctl: &Ctl<'_>)
    -> Result<BinaryOutcome, VfsError>;

/// `ContentComparator` for the CRC method.
#[derive(Debug, Clone, Copy)]
pub struct CrcComparator { pub opts: StreamOptions }
/// `ContentComparator` for the binary method.
#[derive(Debug, Clone, Copy)]
pub struct BinaryComparator { pub opts: StreamOptions }
impl ContentComparator for CrcComparator { /* BinarySame | BinaryDifferent | Error */ }
impl ContentComparator for BinaryComparator { /* BinarySame | BinaryDifferent | Error */ }

/// A file reachable through a `Vfs`, as a `ContentSource`.
pub struct VfsSource<'a> { vfs: &'a dyn Vfs, spec: Spec, entry: Entry, local: OnceLock<Option<PathBuf>> }
impl<'a> VfsSource<'a> {
    /// Builds a source (`entry` must describe `spec`). Makes no back-end call.
    pub fn new(vfs: &'a dyn Vfs, spec: Spec, entry: Entry) -> VfsSource<'a>;
}
impl ContentSource for VfsSource<'_> {
    /* name = entry name; len = entry size (files); known_crc32 = entry.crc32; local_path = Vfs::local_path for
       files, asked on first use and cached; open = Vfs::open_read; read_all = exact-size read via open_read with
       a cancellation check per 1 MiB and between partial reads; cancellation carried by an error (a
       `VfsError::Cancelled` from open_read, a reader error that wraps `Cancelled`) → `ReadError::Cancelled`; a
       reported size ≤ 4096 bytes is not trusted for the `limit` check (§2.20 step 2) */
}
```

#### 1.2.6 The `mmap` module: the only `unsafe` in library and program code

```rust
// core/hikasa-vfs/src/mmap.rs   (lib.rs: `#[allow(unsafe_code)] mod mmap;`)
// Public surface is crate-private and safe. Mapped slices never leave this module.

/// Compares two local regular files by mapping them one chunk at a time, on `threads` threads (the calling
/// thread plus scoped helpers, §5.1; the outcome does not depend on the count). Returns `Ok(None)` when either
/// cannot be mapped (empty file, not a regular file, a chunk below the first difference whose mapping or
/// pre-faulting is refused) **or when a re-stat after the comparison shows that the length or the
/// modification time of either file changed**; the caller then streams (§2.20).
pub(crate) fn compare_files_mapped(a: &Path, b: &Path, chunk: usize, threads: usize, ctl: &Ctl<'_>)
    -> std::io::Result<Option<BinaryOutcome>>;
/// CRC32 of a mapped local regular file (`Ok(None)` as above, including the re-stat check); the chunk CRCs are
/// combined in offset order with `crc32fast::Hasher::combine`.
pub(crate) fn crc32_file_mapped(path: &Path, chunk: usize, threads: usize, ctl: &Ctl<'_>)
    -> std::io::Result<Option<u32>>;

fn map_read_only(file: &File, offset: u64, len: usize) -> std::io::Result<memmap2::Mmap> {
    // SAFETY: `memmap2::MmapOptions::map` is unsafe because another process may modify or truncate the file
    // while it is mapped. A modification can change the slice's bytes; a truncation can raise SIGBUS.
    // Mitigations: (1) the map is read-only and private to this module; (2) it is created only for
    // local *regular* files that `metadata()` confirms after opening; (3) the bytes are only compared
    // or hashed in bounded chunks, so changed bytes can make the *result* stale but cannot break memory
    // safety elsewhere; (4) a mapping covers one chunk and lives only while that chunk is processed, inside
    // one call; (5) after the call the file is re-stat'ed, and a changed length or mtime discards the mapped
    // result in favour of streaming; (6) on Linux each chunk is pre-faulted with MADV_POPULATE_READ before it
    // is read, which fails instead of raising SIGBUS when a truncation has cut the chunk off, and the attempt
    // is then abandoned; (7) helper threads are scoped (`thread::scope`): each one maps, reads and drops its
    // own chunk mappings, and all of them end before the call returns.
    // Residual risk (accepted, documented in docs/11 §3): a truncation between populating a chunk and
    // reading it (or any truncation on other systems, or on Linux before 5.14) can terminate the process.
    // MmapPolicy::Never disables this path.
    unsafe { memmap2::MmapOptions::new().offset(offset).len(len).map(file) }
}
```

The architecture test (§6.1) fails if the `unsafe` keyword (regex `\bunsafe\s*(\{|fn\b|impl\b|trait\b|extern\b)`)
appears in any `.rs` file under `core/`, `cli/` or `gui/` other than `core/hikasa-vfs/src/mmap.rs` and the
allocation-counting test and bench binaries, or if an allowed occurrence has no preceding `// SAFETY:` comment.
The allocation-counting exemption (§6.1 check 1) covers exactly the files in a crate's own `tests/` or `benches/`
directory (`<top>/<crate>/tests/…`, `<top>/<crate>/benches/…`) whose code installs a `#[global_allocator]`:
`GlobalAlloc` is an `unsafe` trait, so a counting allocator cannot be written without `unsafe`. Such a file says
`#![allow(unsafe_code)]` (the workspace lint is `deny`), and each of its `unsafe` occurrences has a preceding
`// SAFETY:` comment. Library code never qualifies (not even a `src/tests/` module): every library crate keeps
`#![forbid(unsafe_code)]`, and `hikasa-vfs` keeps `#![deny(unsafe_code)]` with the single `mmap` allow, so
`mmap.rs` remains the only `unsafe` in library and program code.

#### 1.2.7 Accepted additions and deviations (wave 2)

The review of wave 2 accepted the following public items, field additions, deviations and clarifications. They
are part of the contract: later waves may use them and must not assume the text above where an item here
refines it.

**Additive API** (no signature above changes):

```rust
impl VfsError {
    /// The most specific variant for an I/O failure of `op` on `spec`: not found, permission denied, not a
    /// folder, is a folder and already exists get their variants; an error that wraps `Cancelled` becomes
    /// `Cancelled`; an error that wraps a `VfsError` is unwrapped; everything else is `Io`.
    pub fn from_io(spec: impl Into<String>, op: VfsOp, error: std::io::Error) -> VfsError;
    /// The spec the error is about (`None` for `InvalidName`, `InvalidAttributes` and `Cancelled`).
    pub fn spec(&self) -> Option<&str>;
    /// `Classify::kind(self) == cancelled`.
    pub fn is_cancelled(&self) -> bool;
}
impl From<Cancelled> for VfsError { /* VfsError::Cancelled */ }
impl From<VfsError> for std::io::Error { /* io_error_kind(result) == kind(error); from_io recovers the original */ }
impl VfsOp     { pub const ALL: [VfsOp; 7]; pub const fn as_str(self) -> &'static str; }     // serde names
impl EntryKind { pub const ALL: [EntryKind; 4]; pub const fn as_str(self) -> &'static str; } // serde names
// Display: EntryKind (as_str), FileName (as_str).
impl Attrs { pub fn dos(&self) -> Option<DosAttrs>; pub fn unix(&self) -> Option<UnixMeta>; }
impl Entry {
    /// Name and kind, no metadata: size 0, no times, `Attrs::None`, no link target, not followed, no CRC, no id.
    pub fn new(name: FileName, kind: EntryKind) -> Entry;
    /// The exact symlink target: `link_target_raw` when the text is lossy, else `link_target`.
    pub fn link_target_os(&self) -> Option<OsString>;
}
impl Capabilities {
    /// The set flags as snake_case names in bit order, the JSON form of bit sets (§4.1 rule 9).
    pub fn names(self) -> Vec<&'static str>;
    /// Inverse of `names` for one name (`None` for unknown names).
    pub fn parse_name(name: &str) -> Option<Capabilities>;
}
impl Listing {
    /// Sorts `entries`, `name_only` and `errors` by name bytes, then exact OS name (errors then by message): the
    /// order of `hikasa vfs ls` (§4.3.4); total, so independent of the back-end's order.
    pub fn sort_by_name(&mut self);
    /// Distinct names: described + name-only + failed names that are not also described (item 2 below).
    pub fn len(&self) -> usize;
    /// True when all three lists are empty.
    pub fn is_empty(&self) -> bool;
}
impl LocalVfs { pub fn options(&self) -> LocalOptions; }
impl MemoryVfs {
    /// Replaces the capabilities reported for every local spec.
    pub fn set_capabilities(&self, caps: Capabilities);
    /// Removes every fault.
    pub fn clear_faults(&self);
    /// Metadata reads so far: `stat` calls plus the per-entry reads of `list` (declined entries are not read).
    pub fn stat_calls(&self) -> u64;
    /// The directory record of the entry at `path` states no kind (like `DT_UNKNOWN`): `list` always stats it
    /// and never passes it to the prefilter (the §6.2.2 test "unknown kinds are stat'ed").
    pub fn set_kind_unknown(&self, path: &str) -> Result<(), VfsError>;
}
impl VfsRouter { pub fn local(&self) -> &LocalVfs; }
impl<'a> VfsSource<'a> { pub fn spec(&self) -> &Spec; pub fn entry(&self) -> &Entry; }
impl StreamOptions {
    pub const DEFAULT_BUFFER_SIZE: usize = 1 << 20;
    pub const MIN_BUFFER_SIZE: usize = 4 << 10;
    pub const MAX_BUFFER_SIZE: usize = 64 << 20;
    pub const MAPPED_CHUNK: usize = 8 << 20;     // mapped chunk between two cancellation checks
    pub const MAX_THREADS: usize = 16;
    pub const UNRELIABLE_SIZE_MAX: u64 = 4096;   // §2.20 step 2
    pub fn effective_buffer_size(&self) -> usize; // buffer_size clamped to [MIN_BUFFER_SIZE, MAX_BUFFER_SIZE]
    pub fn effective_threads(&self) -> usize;     // threads clamped to [1, MAX_THREADS]
}
impl MmapPolicy { pub const DEFAULT_MIN_LEN: u64 = 16 << 20; pub fn maps(self, len: u64) -> bool; }
impl BinaryOutcome { pub fn is_same(self) -> bool; }
```

`Blob` implements `Debug` by hand (its length, not its bytes); `MemoryVfs` implements `Debug` (counts) and
`Default` (case-sensitive) by hand. The trait sets are those of §1.2.4.

**Field additions** (breaking for struct literals; made in wave 2, before any consumer existed; literals use
`..StreamOptions::default()` and `Entry::new`):

- `StreamOptions::threads: usize`: threads of one mapped CRC or comparison (§1.2.5, §2.20, §5.1). Default 1.
- `Entry::link_target_raw: Option<OsString>`: the exact target when `link_target` is lossy (item 9).

**Deviations and clarifications:**

1. `VfsError::InvalidAttributes { input }` belongs to the §1.2.3 enum: the error of `DosAttrs::parse_letters`
   (`Classify`: `invalid_input`).
2. **Listings are not disjoint.** With `FollowLinks::Yes`, a symlink whose target cannot be stat'ed (a broken
   link, a loop) is listed unfollowed in `Listing::entries` **and** with an `EntryError` of the same name in
   `Listing::errors` (`LocalVfs` and `MemoryVfs`). The folder scanner (§1.6) attaches the error to that entry
   (one node with the side's error flag) and never creates a second node for the name. `Listing::len` counts
   the name once.
3. **Prefilter on `LocalVfs`.** The kind comes from `std::fs::DirEntry::file_type`, which reads `d_type` (or the
   Windows find data) and `lstat`s the entry when the record states no kind (`DT_UNKNOWN`). Such an entry is then
   passed to the prefilter with the kind found and, when declined, reported name-only although it was stat'ed.
   Verdicts are unchanged; only the D-45 saving is lost on those file systems (std reads no `d_type` without the
   fallback, and `libc` is not approved, §0.7). `MemoryVfs` keeps the rule "unknown kinds are stat'ed and never
   passed to the prefilter" (`set_kind_unknown`).
4. **Router texts.** `open_write` on `Spec::Stdin` is `Unsupported { what: "writing to standard input" }`, on
   `Spec::Clipboard` `Unsupported { what: "writing to the clipboard" }` (with or without a blob). A blob entry is
   named after the spec's canonical text, `-` or `clipboard://`, as a root entry carries its path text; such a
   name is not a valid `FileName::new` input, and `ContentSource::name` returns it (format detection uses the
   name hint, D-15).
5. **`MemoryVfs` identities and faults.** `Entry::file_id` is `Some(FileId { dev: 0, ino: node index + 1 })`, so
   D-33 loop tests run on `MemoryVfs`. `Fault { kind: protocol | internal }` gives `VfsError::Io` classified
   `io` (no file system error carries those kinds); `Fault { op: Map }` has no effect (nothing is mapped). A
   symlink loop gives the local file system's error: the OS error `ELOOP` (Linux, macOS, the BSDs) or
   `ERROR_CANT_RESOLVE_FILENAME` (Windows), with the same `std::io::ErrorKind` and message as `LocalVfs`.
6. **Pre-faulting.** On Linux each mapped chunk is pre-faulted with `MADV_POPULATE_READ` (kernel 5.14 or later)
   before it is read. A failure, e.g. after a concurrent truncation, abandons the mapped attempt (`Ok(None)`)
   and the caller streams; a kernel older than 5.14 answers `EINVAL` and the chunks are demand-faulted, with the
   residual risk of other systems (§1.2.6 SAFETY (6), docs/11 §3, docs/02 "mmap residual risk").
7. **Reader placeholders.** `crc32_reader` and `compare_readers` name a failed reader `<reader>`,
   `<first reader>` or `<second reader>` in `VfsError::Io { spec, .. }`; `crc32_source` and `compare_sources`
   name the source's spec instead.
8. **Unreliable sizes** (§2.20 step 2). A reported size of at most 4096 bytes (`StreamOptions::UNRELIABLE_SIZE_MAX`)
   never decides "different" by itself — in `compare_sources`, in `CrcComparator` and in the `read_all` limit
   check — because procfs reports 0 and sysfs 4096 for files of other lengths; such files are read. `Entry::size`
   stays what the OS reports, so metadata comparisons (the SIZE quick test) see the reported value.
9. **Symlink sizes and exact targets.** An unfollowed symlink's `size` is the byte length of the exact target
   that `read_link` returned, on every platform (procfs links report 0 in `lstat`; Windows reports 0).
   `link_target` is UTF-8 (lossy when needed) and `link_target_raw` keeps the exact target when it is lossy, so
   two targets that differ only in bytes that are not UTF-8 are different entries. Exact comparisons of link
   targets (the LINK_TARGET quick test, §2.6.1) compare `Entry::link_target_os()`, not the lossy text.
10. **Lazy local path.** `VfsSource::new` makes no back-end call. `ContentSource::local_path` asks
    `Vfs::local_path` (a `stat` on `LocalVfs`) on first use and caches the answer; `crc32_source` asks only when
    the mmap policy would map the file's length.
11. **Cancellation and progress** (§5.3, §5.4). The streaming loops (and `read_all`) check the token between the
    partial reads that fill one buffer, so a slow reader delays cancellation by at most one `read` call (a single
    blocking `read` is not interruptible). `MemoryVfs::list` checks it every 1024 entries while filtering and
    while describing. Every finished CRC or comparison (streamed or mapped) ends with one unconditional
    completion report (`Ctl::progress_now`, `done == total` = the bytes processed: the length, or the offset that
    decided), so a throttled progress bar reaches its end. Mapped helper threads never report; the calling
    thread reports the shared byte count, so `done` never decreases.
12. **Write targets that are links or not regular files** (wave-5 review, maintainer decision 4; `LocalVfs`
    only). `open_write` on a symbolic link resolves the chain (relative targets against the link's folder, at
    most 40 links; a loop is the OS's error): the file the link points to is replaced atomically, with the
    temporary file created next to it, and the link stays; a dangling link creates the file it names; a link to
    a folder is `IsADirectory`. A target that exists and is not a regular file (a FIFO, a character or block
    device such as `/dev/null`, a socket) is **never replaced**: it is opened for writing and written to
    directly, which is not atomic (what was written before a failure or a drop has reached it), and `commit`
    flushes without syncing. `overwrite = false` still refuses every existing target, links included. The
    `AtomicWrite` contract of §1.2.3 holds for every regular-file target. `MemoryVfs` is unchanged.

### 1.3 hikasa-diff

**Purpose.** Pure token-sequence alignment (ARCH-002). The crate does no I/O and knows nothing about text
or grammars: it aligns two sequences of `TokenId` with optional weights and a set of "trivial" tokens. It
also provides inline (within-line) differences between two strings and a bigram similarity measure.
**Uses.** hikasa-core, imara-diff, serde, thiserror, smallvec. Dev-only: `similar`, as an LCS oracle.

#### 1.3.1 Module tree

```
core/hikasa-diff/src/
├── lib.rs
├── intern.rs        TokenId, Interner<K>, intern_pair, MAX_SEQUENCE_LEN
├── dense.rs         DenseRemap (crate-private; mandatory before every imara-diff call, §2.14.3)
├── model.rs         RunKind, Run, Alignment, RowRef, RunSink
├── options.rs       Algorithm, AlignOptions, Closeness, WorkBudget
├── input.rs         AlignInput, TrivialSet, PairScorer
├── algo/
│   ├── mod.rs       align() dispatcher, prefix/suffix stripping, gap pairing (§2.15)
│   ├── unaligned.rs
│   ├── lcs.rs       imara-diff adapter (Myers, Histogram): DenseRemap, helper thread for > 4,096 tokens
│   │                per side and the cap on abandoned helpers (D-54), hunk → run conversion
│   ├── fallback.rs  Standard's bounded fallback: Histogram within the fallback budget, else the unique-line
│   │                split and gap pairing (§2.14.5, D-16)
│   ├── patience.rs  unique-line anchors + LIS, Myers fallback in gaps
│   ├── standard.rs  staged anchors, two-diagonal skew window, weights, budget (§2.14.5)
│   └── lis.rs       weighted LIS with a Fenwick tree (deterministic tie-breaking)
├── closeness.rs     monotone similarity pairing (DP), budgets
├── similarity.rs    BigramProfile, bigram_similarity (integer per-mille)
├── inline.rs        InlineOptions, InlineDiff, inline_diff (segments → chars)
├── classify.rs      split_ranges_by_class (generic importance splitting for inline ranges)
└── error.rs         DiffError
```

#### 1.3.2 Public API

```rust
/// An interned token (a line key, a segment, a char …). Ids are dense, starting at 0, assigned in
/// first-occurrence order over left-then-right input (INV-INTERN-1: deterministic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenId(pub u32);

/// Interner. The `HashMap` is internal and never iterated (T1).
#[derive(Debug, Clone)]
pub struct Interner<K: Hash + Eq> { map: HashMap<K, TokenId>, next: u32 }

impl<K: Hash + Eq> Interner<K> {
    /// Empty interner.
    pub fn new() -> Self;
    /// With capacity.
    pub fn with_capacity(n: usize) -> Self;
    /// Id of `key`, assigning the next id when new. Fails with `InputTooLarge` beyond `i32::MAX - 1` ids
    /// (imara-diff's limit).
    pub fn intern(&mut self, key: K) -> Result<TokenId, DiffError>;
    /// Number of distinct keys.
    pub fn len(&self) -> u32;
    /// True when empty.
    pub fn is_empty(&self) -> bool;
}

/// Left and right token sequences that share one interner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternedPair {
    /// Left tokens.
    pub left: Vec<TokenId>,
    /// Right tokens.
    pub right: Vec<TokenId>,
    /// Number of distinct tokens.
    pub distinct: u32,
}

/// Interns two byte-key sequences (left first). The keys are borrowed (hikasa-text passes slices of its
/// per-chunk key arenas), so interning copies no key bytes. Fails with `InputTooLarge` when either sequence
/// is longer than `MAX_SEQUENCE_LEN`.
pub fn intern_pair<'k>(
    left: impl IntoIterator<Item = &'k [u8]>,
    right: impl IntoIterator<Item = &'k [u8]>,
) -> Result<InternedPair, DiffError>;

/// Longest sequence any engine accepts: `i32::MAX - 2` (imara-diff asserts `len < i32::MAX`). hikasa-text
/// rejects longer documents earlier (D-43), so no user input reaches the assertion (T9).
pub const MAX_SEQUENCE_LEN: u32 = i32::MAX as u32 - 2;

/// Crate-private, listed because it is a performance obligation (D-16): remaps the token ids of a sub-range
/// pair to a dense local alphabet `0..k` before every imara-diff call, because imara-diff allocates
/// O(max token id) per call (Myers `token_occurrences`, `Histogram::new(num_tokens)`). One instance per
/// `align` call; epoch-stamped, so resetting is O(1).
pub(crate) struct DenseRemap { stamp: Vec<u32>, local: Vec<u32>, epoch: u32 }

impl DenseRemap {
    /// Capacity for global ids `< distinct` (O(distinct) once per alignment).
    pub(crate) fn new(distinct: u32) -> DenseRemap;
    /// Writes the remapped tokens of `a` and `b` (first-occurrence order, `a` before `b`) and returns the local
    /// alphabet size `k`, which is passed to imara-diff as `num_tokens`. O(|a| + |b|).
    /// `debug_assert!` every written id is `< k`.
    pub(crate) fn remap(&mut self, a: &[TokenId], b: &[TokenId],
                        out_a: &mut Vec<imara_diff::Token>, out_b: &mut Vec<imara_diff::Token>) -> u32;
}

/// Run kinds of an alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// Pairs with equal tokens (one row per pair).
    Equal,
    /// Pairs with different tokens (one row per pair; 1:1, equal lengths).
    Changed,
    /// Left-only tokens.
    Delete,
    /// Right-only tokens.
    Insert,
}

/// A run: `left` and `right` are half-open index ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Run {
    /// Kind.
    pub kind: RunKind,
    /// Left range.
    pub left: Range<u32>,
    /// Right range.
    pub right: Range<u32>,
}

/// One row of an alignment expanded from its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowRef {
    /// Kind of the run it came from.
    pub kind: RunKind,
    /// Left index (None for `Insert`).
    pub left: Option<u32>,
    /// Right index (None for `Delete`).
    pub right: Option<u32>,
}

/// The result of an alignment. Immutable; invariants INV-ALIGN-1..7 (Appendix B).
/// The stable id of a run is its index; ids are deterministic for identical inputs and options (ARCH-050).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alignment { runs: Vec<Run>, left_len: u32, right_len: u32, used: Algorithm, warnings: Vec<Warning> }

impl Alignment {
    /// Runs in document order.
    pub fn runs(&self) -> &[Run];
    /// Left sequence length.
    pub fn left_len(&self) -> u32;
    /// Right sequence length.
    pub fn right_len(&self) -> u32;
    /// Rows = Σ max(|left|, |right|) over runs (Delete/Insert contribute their own length).
    pub fn row_count(&self) -> u64;
    /// Expands runs into rows, in order.
    pub fn rows(&self) -> impl Iterator<Item = RowRef> + '_;
    /// Algorithm requested. A Standard alignment whose budget ran out still reports `Standard`; the fallback
    /// shows up as warning `work_budget_exhausted`.
    pub fn algorithm(&self) -> Algorithm;
    /// Warnings.
    pub fn warnings(&self) -> &[Warning];
    /// Checks INV-ALIGN-1..7 against the inputs.
    pub fn check_invariants(&self, input: &AlignInput<'_>, opts: &AlignOptions) -> Result<(), InvariantViolation>;
    /// Reconstructs the right sequence from left + inserted tokens (property "apply(diff, left) = right").
    pub fn apply<T: Clone>(&self, left: &[T], right: &[T]) -> Vec<T>;
}

/// Receives runs as soon as they are final, in document order (progressive output, ARCH-004).
pub trait RunSink {
    /// Called with consecutive, non-overlapping runs.
    fn runs_ready(&mut self, runs: &[Run]);
}

/// Alignment engine (TC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm { Unaligned, Standard, Myers, Patience, Histogram }

/// Closeness matching parameters (TC-011; §2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Closeness {
    /// Minimum similarity (per mille, 0..=1000) for two lines to pair. Default 500.
    pub threshold_permille: u16,
    /// Per-gap DP cell cap (p·q); larger gaps pair by position. Default 4_000_000.
    pub max_cells_per_gap: u64,
    /// Total cell budget for one alignment (`None` = 32·(n+m) + 4_000_000).
    pub max_cells_total: Option<u64>,
}

/// Alignment options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlignOptions {
    /// Engine.
    pub algorithm: Algorithm,
    /// Skew tolerance in lines (Standard; TC-011). Default 300. `u32::MAX` = unlimited.
    pub skew_tolerance: u32,
    /// Closeness, applied to the gaps of Standard, Histogram and Patience (D-17). Unaligned and Myers pair by
    /// position; Myers adds warning `closeness_ignored` when this is `Some`.
    pub closeness: Option<Closeness>,
    /// Never pair differing tokens: gaps become Delete + Insert (TC-011).
    pub never_align_differences: bool,
    /// Standard work budget in elementary operations (`None` = 64·(n+m) + 1_000_000; §2.14.5).
    pub work_budget: Option<u64>,
}

/// A dense bit set of token ids that are "trivial" (cannot act as anchors; e.g. blank or all-unimportant
/// lines). Built by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrivialSet { bits: Vec<u64> }

impl TrivialSet {
    /// Set able to hold ids `< distinct`.
    pub fn with_capacity(distinct: u32) -> TrivialSet;
    /// Marks `id`.
    pub fn insert(&mut self, id: TokenId);
    /// Tests `id`.
    pub fn contains(&self, id: TokenId) -> bool;
}

/// Inputs of an alignment.
#[derive(Debug, Clone, Copy)]
pub struct AlignInput<'a> {
    /// Left keys.
    pub left: &'a [TokenId],
    /// Right keys.
    pub right: &'a [TokenId],
    /// Left line weights 0..=4 (empty slice = all 0). Length must equal `left.len()` otherwise.
    pub left_weights: &'a [u8],
    /// Right line weights (same rules).
    pub right_weights: &'a [u8],
    /// Trivial tokens.
    pub trivial: &'a TrivialSet,
    /// Number of distinct tokens (from the interner).
    pub distinct: u32,
}

/// Similarity of two lines for closeness matching, per mille (0..=1000).
pub trait PairScorer: Sync {
    /// Similarity of left line `l` and right line `r` (indices into the full sequences).
    fn similarity(&self, l: u32, r: u32) -> u16;
}

/// Aligns two sequences (§2.14–2.15). Pure; cancellation is checked at least every 65 536 elementary steps.
/// An imara-diff call on at most 4,096 tokens per side runs inline (cancellation checked before and after);
/// a larger call runs on a helper thread that owns copies of the remapped tokens, while the caller polls
/// `cancel` every 10 ms (`Receiver::recv_timeout`) and returns `Err(Cancelled)` at once, discarding the
/// helper's result; while 2 abandoned helpers are still running, a new helper call first waits for one of them
/// (D-54, §1.3.3). Results never depend on which path ran (T6). Standard is bounded (O(n log n) plus its
/// budgets, §2.14.5); Myers, Histogram and Patience have no budget.
pub fn align(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    scorer: Option<&dyn PairScorer>,
    sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
) -> Result<Alignment, DiffError>;

/// A multiset of character bigrams (sorted `u64` codes) for bigram similarity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BigramProfile { grams: Vec<u64>, chars: u32 }

impl BigramProfile {
    /// Profile of `text` (by `char`s).
    pub fn new(text: &str) -> BigramProfile;
}

/// Dice coefficient on bigram multisets, per mille, integer arithmetic (§2.15.3).
pub fn bigram_similarity(a: &BigramProfile, b: &BigramProfile) -> u16;

/// Inline diff options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineOptions {
    /// Lines longer than this (chars, either side) are reported as one whole-line change. Default 10_000.
    pub max_line_chars: u32,
    /// Changed segment hunks up to this many chars (both sides together) are refined per char. Default 512.
    pub refine_max_chars: u32,
}

/// Differing byte ranges within a line pair (INV-INLINE-1..4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineDiff {
    /// Sorted, disjoint, non-empty byte ranges on char boundaries of the left text.
    pub left: Vec<Range<u32>>,
    /// Same for the right text.
    pub right: Vec<Range<u32>>,
    /// True when the line exceeded `max_line_chars` (ranges then cover the whole differing middle).
    pub whole_line: bool,
}

/// Character/segment-level difference of two strings (§2.16). Pure and deterministic.
pub fn inline_diff(left: &str, right: &str, opts: &InlineOptions) -> InlineDiff;

/// Splits byte ranges at class boundaries: `class_of(byte_offset)` returns a class id; output ranges carry the
/// class of all their bytes. Used by hikasa-text to colour unimportant parts blue (TC-014).
pub fn split_ranges_by_class(ranges: &[Range<u32>], class_of: &dyn Fn(u32) -> u8) -> Vec<(Range<u32>, u8)>;

/// Diff errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DiffError {
    /// A sequence longer than `MAX_SEQUENCE_LEN`, or more than `i32::MAX - 1` distinct tokens.
    #[error("input too large: {tokens} tokens")]
    InputTooLarge { tokens: u64 },
    /// Weights slice length mismatch (caller bug).
    #[error("weights length {got} does not match sequence length {expected}")]
    WeightsLength { expected: usize, got: usize },
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
}
```

#### 1.3.3 Accepted additions and deviations (wave 2)

The review of wave 2 accepted the following public items, deviations and clarifications of hikasa-diff. They are
part of the contract: hikasa-text (wave 3) and later waves may use them and must not assume the text above where
an item here refines it. No signature of §1.3.2 changes.

**Additive API** (no signature above changes):

```rust
impl Algorithm {
    pub const ALL: [Algorithm; 5];                              // declaration order
    pub const fn as_str(self) -> &'static str;                  // the serde names; Display writes them
}
impl RunKind { pub const fn as_str(self) -> &'static str; }     // the serde names
impl Run { pub fn rows(&self) -> u32; }                         // max(|left|, |right|)
impl Closeness {
    pub const DEFAULT: Closeness;                               // 500 ‰, 4,000,000 cells per gap, automatic total
    pub fn total_cells(&self, n: usize, m: usize) -> u64;       // max_cells_total, or 32·(n+m) + 4,000,000
}
impl AlignOptions {
    /// The D-04 defaults for an explicit algorithm: skew 300, closeness on (500 ‰), never-align off, automatic
    /// work budget.
    pub const fn with_algorithm(algorithm: Algorithm) -> AlignOptions;
    pub fn work_budget_for(&self, n: usize, m: usize) -> u64;   // work_budget, or 64·(n+m) + 1,000,000
    pub fn fallback_budget_for(n: usize, m: usize) -> u64;      // 128·(n+m) + 32,000,000 cells (item 3)
}
impl InlineOptions { pub const DEFAULT: InlineOptions; }        // 10,000 chars per line, refinement ≤ 512 chars
// Default: Closeness (DEFAULT), InlineOptions (DEFAULT), Interner (new), AlignOptions (item 7).
impl BigramProfile { pub fn char_count(&self) -> u32; pub fn bigram_count(&self) -> usize; }
impl<F: Fn(u32, u32) -> u16 + Sync> PairScorer for F { /* calls the closure */ }
impl From<Cancelled> for DiffError { /* DiffError::Cancelled */ }
// Classify for DiffError: InputTooLarge → too_large; WeightsLength (a caller bug) → internal; Cancelled → cancelled.

/// Codes of the warnings that `hikasa_core::WarningCode` does not have yet; the snake_case names are the stable
/// codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiffWarningCode { AlignmentDegraded }                  // as_str / Display: "alignment_degraded"
/// `hikasa_core::Warning` without the spec (an alignment concerns no spec).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiffWarning { pub code: DiffWarningCode, pub message: String }
impl Alignment {
    /// Warnings whose codes `hikasa_core::WarningCode` lacks (each code at most once).
    pub fn diff_warnings(&self) -> &[DiffWarning];
}

/// Test support, **not a stable API** (`#[doc(hidden)]`, may change in any release): public only because timing
/// tests must live outside `src/` (§6.1 check 2).
#[doc(hidden)]
pub mod testing {
    /// `align` with the D-54 helper threshold set to `helper_threshold` tokens per side (the test-only option of
    /// §6.2.3; 0 runs every imara-diff call on a helper).
    pub fn align_with_helper_threshold(
        input: &AlignInput<'_>, opts: &AlignOptions, scorer: Option<&dyn PairScorer>,
        sink: Option<&mut dyn RunSink>, cancel: &CancelToken, helper_threshold: usize,
    ) -> Result<Alignment, DiffError>;
    /// The production threshold (4,096).
    pub const HELPER_THRESHOLD_TOKENS: usize;
    /// `align` with Standard's fallback budget set to `fallback_budget` cells instead of
    /// `AlignOptions::fallback_budget_for(n, m)`, so that small inputs reach the degraded split (P-ALIGN-10).
    pub fn align_with_fallback_budget(
        input: &AlignInput<'_>, opts: &AlignOptions, scorer: Option<&dyn PairScorer>,
        sink: Option<&mut dyn RunSink>, cancel: &CancelToken, fallback_budget: u64,
    ) -> Result<Alignment, DiffError>;
    /// Abandoned helpers still running, process-wide (item 2).
    pub fn abandoned_helpers() -> usize;
}
```

The modules `classify`, `error`, `inline`, `input`, `intern`, `model`, `options` and `similarity` are public as
well; every public item is also re-exported at the crate root.

**Upstream change for hikasa-core.** `WarningCode` needs the variant `AlignmentDegraded` (`"alignment_degraded"`:
"Standard alignment's fallback budget ran out; some ranges were aligned at unique lines and paired by position or
similarity", D-16). hikasa-diff does not edit hikasa-core in wave 2: it reports the warning as a `DiffWarning` in
`Alignment::diff_warnings()`, and hikasa-text maps every entry to the core `Warning` of the same code (same
message, `spec: None`) once the variant exists.

**Deviations and clarifications:**

1. **Helper threshold 4,096 tokens per side** (D-54, §1.3.1, §1.3.2, §2.14.3, §5.1; was 200,000). imara-diff is
   roughly quadratic on adversarial input (random tokens over 50 symbols: 40,000 per side ≈ 0.4 s, 80,000 ≈
   1.7 s, so 200,000 ≈ 10 s), and an inline call cannot be cancelled, so 200,000 tokens could not meet the
   100 ms latency of §5.3. The slowest inline calls measured at 4,096 tokens per side (release build; periodic
   sequences against their reversal, random sequences over 2–100 symbols) took 10–16 ms at moderate load on the
   development machine; an independent measurement on the same machine found up to 40 ms (best of three) and
   60–80 ms under heavy load. The margin under 100 ms is therefore about 2.5×. A larger call pays one thread
   spawn (tens of µs). P-ALIGN-7 runs every imara-diff call on a helper (threshold 0) and checks that the result
   is identical.
2. **Abandoned helpers are capped** (D-54, §5.1). imara-diff has no cancellation hook, so the helper of a
   cancelled call computes until its call ends: seconds to minutes on adversarial input for an explicitly chosen
   Histogram or Myers. At most 2 abandoned helpers run per process. A helper call made while 2 are running waits
   for one of them to end, polling its own `CancelToken` every 10 ms (`Condvar::wait_timeout`), so it stays
   cancellable within 100 ms. Abandoned helpers are therefore at most 2 + the number of concurrent `align` calls.
   The count is a process-wide `static Mutex<usize>` with a `Condvar`, the crate's only mutable static: an
   accepted exception to T7, because it decides only *when* a helper call starts, never what any call returns
   (T6). The cost: after two cancellations of slow calls, a new call above the threshold may wait until an
   abandoned helper ends. Standard's own helper calls are bounded by its fallback budget (item 3).
3. **Bounded Standard** (D-16, §2.14.5). "The fallback costs O(n + m) in total" held for DenseRemap's
   allocations, not for time: Histogram is roughly quadratic, so Standard on 1,000,000 random lines over 50
   symbols would have taken about a minute (≈ 6.6 s already at 320,000). After the work budget runs out, a range
   is aligned by Histogram only while its estimated cost `|A|·|B|` fits the fallback budget
   (`AlignOptions::fallback_budget_for`: 128·(n+m) + 32,000,000 cells; imara-diff's Histogram measured ≤ ≈ 1.6 ns
   per cell for every input shape tried). Beyond it, the range is split once at keys unique on both sides (patience
   LIS, anchors extended to blocks), and each gap goes to Histogram if it fits the remaining budget, else to gap
   pairing (§2.15); the diff warning `alignment_degraded` is added. Standard is O((n+m) log(n+m)) plus its budgets
   on every input: 1,000,000 random lines per side over 50 symbols take 0.72 s (release), and the test
   `bounded_standard` asserts < 2 s in release and the growth for ×8 the input in debug. Costs are counted in
   operations, never time, so results stay deterministic and independent of the thread count. Explicitly chosen
   Histogram, Myers and Patience stay exact (never degraded, no budget): their adversarial worst case is
   super-linear (about quadratic), and they stay cancellable.
4. **Stage B/C skip rule** (§2.14.5 step 4). A candidate `(i, j)` is skipped when `(i−1, j−1)` is a match *of the
   same stage* (non-trivial in Stage B, trivial in Stage C), not any match. With the literal rule a block whose
   first line is a matching trivial line (a blank line before repeated code) is never scored by Stage B, and a
   worse block can win: left `["", x, y, e1]`, right `[x, y, "", x, y, e2]` aligned 2 lines instead of 3 (the
   LCS, which Myers, Histogram and Patience find as well). A block started behind a trivial match lies on the
   diagonal of its maximal block, with the same displacement and a lower score, so the rule never makes a worse
   block win. In Stage C both rules coincide. hikasa-text goldens depend on this rule.
5. **P-ALIGN-8 needs work to do.** `work_budget = 0` gives the Histogram result, plus warning
   `work_budget_exhausted` whenever the stripped middle has both sides non-empty (identical inputs or a pure
   insertion need no work and get no warning, as §2.14.5 says "when the budget is exceeded"). Property-sized
   inputs always fit the fallback budget.
6. **Closeness without a scorer** (§2.15.2). With closeness on and `scorer = None`, every pair of different keys
   has similarity 0, so with `threshold_permille ≥ 1` only equal keys pair (with 0, every pair is allowed, as the
   formula says). Weights above 4 count as 4 and scorer values above 1000 as 1000 (totality, T9).
7. **`AlignOptions::default()` is `with_algorithm(Standard)`**, not D-36's `Auto`: `Auto` is resolved by
   hikasa-text from the effective format (D-36), and hikasa-diff has no `Auto` variant.
8. **Myers is not a minimal LCS** (TC-010, §2.14.3). imara-diff's Myers is a heuristic O(ND) diff: its
   preprocessing prunes tokens that occur often in the other side when their neighbours are unmatched, and a
   pruned token never matches. `[a, a]` against `[b, c, a, d, b]` gives no Equal pair although the LCS is 1
   (Histogram finds it). Patience's Myers fallback inherits this. No test claims minimality: P-ALIGN-4 checks
   `Equal pairs ≤ LCS`, and the test `myers_is_not_guaranteed_minimal` pins the example.
9. **Progress** (§5.4). `diff::align` has no progress sink (§1.3.2 gives it only the `CancelToken`), so it emits
   no `aligning` progress; hikasa-text reports that phase from the `RunSink` calls. Standard and Unaligned report
   progressively (Unaligned every 4,096 pairs); Myers and Histogram send the common prefix in a call of its own
   before the imara-diff call, then the runs after each hunk once the call has returned (§1.5
   `CompareObserver::partial`).
10. **Cancellation granularity** (§5.3). Every linear scan over tokens (the dense renumbering of out-of-range ids,
    the common prefix and suffix, block extension, the stage tables) checks the token at least every 65,536
    tokens. The closeness DP checks it every 4,096 cells, also inside one row of a 1 × 4,000,000 gap, and keeps
    its totals along the shorter side of the gap: O(min(p, q)) besides the direction byte per cell (§5.7). Every
    gap counts as one step, so a run of tiny gaps also reaches a check point.
11. **Intern memory** (§0.8). `intern_pair` sizes its table by the distinct keys, not by the line count: it starts
    at 4,096 entries and, after 65,536 keys, reserves room for (distinct keys so far / 65,536) × the longer side.
    Each distinct key costs one std hash-table entry (a 16 B borrowed key, the 4 B id, padding and a control byte:
    25 B per bucket at 44–88 % load), so 28–57 B per distinct key: 30.5 B on the §0.8 1,000,000-line input, where
    pre-sizing by lines gave 61 B, at the same speed. The 24 B of §0.8 is not reachable with borrowed keys: the
    16 B key reference alone leaves 8 B for the id and the table's free slots. A table of `u32` ids with the keys in
    a separate vector measured 29.5 B per key at 1.5–2× the time, so the std table stays (the phase's 150 ms is
    the tighter budget). The table's capacity never influences an id (T1).
12. **`TrivialSet` memory.** The set costs one bit per id up to the largest id inserted (`TokenId(u32::MAX)` →
    512 MiB), so callers insert interner ids (< `distinct`). Ids far beyond 4·(n+m) in the sequences are renumbered
    densely inside `align`; the caller's set is sized by the caller.

### 1.4 hikasa-formats

**Purpose.** The file-format model (FF-001…FF-040): TOML (de)serialisation, grammar compilation, the
tokenizer (FF-026), line weights, the registry (detection and shadowing), built-in presets, encoding
detection and decoding, binary detection, and conversions (None, Sorted, External).
**Uses.** hikasa-core, serde, toml, regex, fancy-regex, encoding_rs, chardetng, thiserror, memchr, rayon.

#### 1.4.1 Module tree

```
core/hikasa-formats/
├── presets/                 FF-040: one TOML file per built-in, embedded with include_str! (order = file prefix)
│   ├── 01-bash.toml          02-c-cpp.toml        03-cobol.toml        04-pascal-forms.toml
│   ├── 05-pascal.toml        06-html.toml         07-java.toml         08-javascript.toml
│   ├── 09-perl.toml          10-python.toml       11-ini-reg.toml      12-sql.toml
│   ├── 13-visual-basic.toml  14-xml.toml          15-rust.toml         16-sorted.toml
│   ├── 17-csv.toml           18-pictures.toml     19-media.toml        20-executables.toml
│   └── 21-registry-files.toml (`Preset::enabled = false`, D-27; every file starts with a `# Sources:` comment)
└── src/
    ├── lib.rs
    ├── model.rs         FileFormat, FormatId, FormatKind, ElementName, ElementInfo, TextSettings, LineWeight,
    │                    EncodingSetting, Conversion, ExternalConversion, FilenameEncoding, TableSettings
    ├── grammar.rs       GrammarItem, ItemKind (+ RawGrammarItem serde shim with precise validation errors)
    ├── toml_io.rs       FORMAT_SCHEMA_VERSION, parse_format, to_toml, unknown_keys
    ├── regex_engine.rs  Pattern, EngineKind, RegexLimits (regex first, fancy-regex when needed; §2.11)
    ├── compile.rs       CompiledFormat, CompiledGrammar, ElementId, ElementTable
    ├── tokenize/
    │   ├── mod.rs       Token, LineState, Tokenizer, LineOutcome (§2.10)
    │   ├── scan.rs      item matchers, first-char prefilter, next-match cache
    │   └── parallel.rs  LineSource, LineBytes, tokenize_all (speculative chunks, per-chunk arenas, §2.10.6)
    ├── weights.rs       line weights (TC-012)
    ├── registry.rs      FormatRegistry, RegistryEntry, Origin, EnabledOverrides, Detection, ShadowStatus
    ├── presets.rs       PRESETS (embedded), FormatRegistry::builtin
    ├── encoding.rs      TextEncoding, DetectedEncoding, EncodingSource, DecodeOptions, detect, decode,
    │                    utf_pattern (shared with binary.rs) (§2.8)
    ├── binary.rs        BinaryVerdict, BinaryReason, MagicKind, sniff (§2.9)
    ├── convert.rs       ProcessRunner, StdProcessRunner, ConversionVars, build_argv, run_external (§2.10.8)
    └── error.rs         FormatError, PatternError, ConvertError
```

#### 1.4.2 Model (persisted as TOML: one file per format, FF-004)

```rust
/// Stable format id: `[a-z0-9][a-z0-9-]{0,63}`. It is also the file stem: `formats/<id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FormatId(String);

/// Grammar element name: free text, trimmed, 1..=64 chars, no `,`. Built-ins use Comment, String, Keyword,
/// Identifier, Number, Operator, Directive, EnvironmentVar, LineNum, Identification, Indent, Whitespace, Char,
/// Attribute, Annotation, Entity, Variable, RegexLiteral (Appendix A). Names compare exactly (case-sensitive);
/// importance rules reference them by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ElementName(String);

/// Kind of view a format selects (FF §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind { Text, Table, Hex, Picture, Media, Version, Registry, External }

/// Per-element settings (FF-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ElementInfo {
    /// Changes in letter case inside this element are important (FF-025). Default true.
    pub case_sensitive: bool,
    /// Default importance of the element's text; `ImportanceRules::elements` overrides it by name (D-04).
    /// Default true. The Python `Whitespace` and COBOL `Identification` elements ship with `false`.
    pub important: bool,
}

/// Text settings (Text and Table kinds).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    /// Tab stop (1..=32). Default 4.
    pub tab_size: u8,
    /// Insert spaces instead of tabs (editing; unused in M1 engines).
    pub tab_as_spaces: bool,
    /// Characters-per-line limit (TC-003); `None` = unlimited.
    pub chars_per_line_limit: Option<u32>,
    /// Lines are independent: every differing row is its own section (FF Misc).
    pub lines_independent: bool,
    /// Column-based data (reserved for M2 table compare).
    pub column_based: bool,
    /// Treat the first Ctrl-Z (0x1A) as end of file (ASCII-compatible encodings only).
    pub ctrl_z_eof: bool,
    /// Save options (TC-005; unused in M1).
    pub save_trim_trailing_ws: bool,
    /// Save options (TC-005; unused in M1).
    pub save_leading_spaces_to_tabs: bool,
    /// Extra characters that count as word characters for List items (word boundaries and keyword lookup,
    /// §2.10.2; D-58). Default empty. COBOL: `"-"`.
    pub word_chars: String,
}

/// A line weight (TC-012): lines matching `pattern` get `priority` (1..=4) as anchor preference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineWeight {
    /// Text or regex searched anywhere in the raw line (no EOL).
    pub pattern: String,
    /// `pattern` is a regex.
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive matching. Default true.
    #[serde(default = "yes")]
    pub match_case: bool,
    /// 1..=4.
    pub priority: u8,
}

/// Encoding setting (FF-032).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EncodingSetting {
    /// BOM → UTF-16/32 pattern → pure ASCII (`fallback`, then locale) → UTF-8 validity → detector (§2.8, D-42).
    Auto {
        /// Label used for content without non-ASCII evidence (pure ASCII); `None` → the adapter's locale
        /// encoding. Ignored when the label's encoding is not ASCII-compatible.
        #[serde(default)]
        fallback: Option<String>,
    },
    /// Fixed code page (a BOM still wins; see §2.8 step 2).
    Fixed {
        /// WHATWG label or "UTF-32LE"/"UTF-32BE".
        label: String,
    },
}

/// Pre-load conversion (FF-030). M1 implements None, Sorted and External.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Conversion {
    /// Plain text.
    #[default]
    None,
    /// Sort lines before comparing (text stage, D-38). The document is read-only.
    Sorted,
    /// External program (byte stage, D-39).
    External(ExternalConversion),
}

/// External conversion command lines (`%s` source, `%t` target, `%o` original).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalConversion {
    /// Load command line (required).
    pub load: String,
    /// Save command line (`None` = converted documents are read-only, FF-031).
    #[serde(default)]
    pub save: Option<String>,
    /// How file names are passed. M1: `Unicode` only; `Ansi` is accepted and treated as `Unicode`.
    #[serde(default)]
    pub filename_encoding: FilenameEncoding,
    /// Seconds before the program is killed (default 60; 0 = no limit).
    #[serde(default = "sixty")]
    pub timeout_secs: u32,
}

/// File name encoding for external programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilenameEncoding {
    /// UTF-8 / UTF-16 native.
    #[default]
    Unicode,
    /// Legacy code page (Windows).
    Ansi,
}

/// Delimited-table settings (parsed and validated now; used by M2 table compare).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableSettings {
    /// Field delimiter (`,` or `\t` …).
    pub delimiter: char,
    /// Text qualifier (`"`).
    #[serde(default)]
    pub qualifier: Option<char>,
    /// Treat consecutive delimiters as one.
    #[serde(default)]
    pub consecutive_as_one: bool,
}

/// A file format (FF §1). Field order is the canonical TOML key order written by `to_toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFormat {
    /// Must be ≤ FORMAT_SCHEMA_VERSION (SES-052).
    pub schema_version: u32,
    /// Stable id.
    pub id: FormatId,
    /// Display name ("C, C++, C#, ObjC Source").
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Kind.
    pub kind: FormatKind,
    /// `;`-separated name masks; empty = manual selection only (FF-001).
    #[serde(default)]
    pub mask: String,
    /// Conversion.
    #[serde(default)]
    pub conversion: Conversion,
    /// Encoding.
    #[serde(default = "auto_encoding")]
    pub encoding: EncodingSetting,
    /// Text settings.
    #[serde(default)]
    pub text: TextSettings,
    /// Element settings by name (an unordered map: `ElementId`s follow first use in `grammar`, D-41).
    #[serde(default)]
    pub elements: BTreeMap<ElementName, ElementInfo>,
    /// Grammar items, ordered (first match wins, FF-026).
    #[serde(default)]
    pub grammar: Vec<GrammarItem>,
    /// Line weights.
    #[serde(default, rename = "line_weight")]
    pub line_weights: Vec<LineWeight>,
    /// Table settings (Table kind).
    #[serde(default)]
    pub table: Option<TableSettings>,
}
```

#### 1.4.3 Grammar

```rust
/// A grammar item: an element name plus a category (FF-020…FF-024).
/// TOML: `[[grammar]] element = "Comment"  kind = "delimited"  from = "/*"  to = "*/"`.
/// Deserialised through `RawGrammarItem` (all fields optional), then validated by `TryFrom`, so that a field
/// that is not allowed for a kind is a precise error ("field `tokens` is not allowed for kind `basic`").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawGrammarItem", into = "RawGrammarItem")]
pub struct GrammarItem {
    /// Element produced. Names not declared in `FileFormat::elements` get the default `ElementInfo`
    /// (`case_sensitive = true`, `important = true`). Ids follow D-41.
    pub element: ElementName,
    /// Category and parameters.
    pub kind: ItemKind,
}

/// Grammar item categories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// FF-020: literal or regex, anchored at the scan position.
    Basic { text: String, regex: bool, match_case: bool },
    /// FF-021: `from` … `to`, or to end of line when `to` is `None` or `stop_at_eol` is set; `escape` skips the
    /// next char (doubled-delimiter rule, §2.10.3).
    Delimited { from: String, to: Option<String>, stop_at_eol: bool, escape: Option<char>, regex: bool, match_case: bool },
    /// FF-022: whole-word tokens, longest match wins.
    List { tokens: Vec<String>, regex: bool, match_case: bool },
    /// FF-023: 1-based inclusive columns; `to_column = None` = to end of line.
    Columns { from_column: u32, to_column: Option<u32> },
    /// FF-024: a heading line and the next `next_lines` lines; `or_line_one` also starts at line 1.
    Lines { heading: String, regex: bool, match_case: bool, next_lines: u32, or_line_one: bool },
}
```

#### 1.4.4 Compilation, regex engines, tokenizer

```rust
/// Regex resource limits (user patterns are untrusted input; T9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegexLimits {
    /// `regex` compiled size limit in bytes. Default 8 MiB.
    pub size_limit: usize,
    /// `fancy-regex` backtracking step limit. Default 1_000_000 (exceeding it = no match + warning).
    pub backtrack_limit: usize,
}

/// Which engine compiled a pattern (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind { Standard, Fancy }

/// A compiled user pattern.
#[derive(Debug, Clone)]
pub struct Pattern { /* source, engine, compiled regex::Regex or fancy_regex::Regex */ }

impl Pattern {
    /// Compiles: `regex` first; when `regex` rejects the pattern, `fancy-regex` is tried (§2.11).
    pub fn compile(source: &str, case_insensitive: bool, limits: &RegexLimits) -> Result<Pattern, PatternError>;
    /// Engine used.
    pub fn engine(&self) -> EngineKind;
    /// Leftmost-first match starting at or after byte `start` (`^`, `\b` see the full haystack).
    /// `Err` only for fancy-regex runtime limits.
    pub fn find_at(&self, haystack: &str, start: usize) -> Result<Option<Range<usize>>, PatternError>;
}

/// Index of an element in a compiled format: 0 = "Everything Else"; then first use in `grammar`; then unused
/// declared elements by name (D-41, INV-FMT-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(pub u16);

impl ElementId {
    /// Text not covered by any grammar item.
    pub const EVERYTHING_ELSE: ElementId = ElementId(0);
}

/// Element names and case flags of a compiled format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementTable { names: Vec<ElementName>, case_sensitive: Vec<bool>, important: Vec<bool> }

impl ElementTable {
    /// Name of `id` ("Everything Else" for 0).
    pub fn name(&self, id: ElementId) -> &str;
    /// Case sensitivity of `id` (id 0: true; the rules decide case for everything-else text).
    pub fn case_sensitive(&self, id: ElementId) -> bool;
    /// Default importance of `id` from `ElementInfo::important` (id 0: true; the rules decide).
    pub fn default_important(&self, id: ElementId) -> bool;
    /// Id by name.
    pub fn id(&self, name: &str) -> Option<ElementId>;
    /// Number of ids (including 0).
    pub fn len(&self) -> u16;
}

/// The compiled grammar (matchers + first-char prefilter).
#[derive(Debug, Clone)]
pub struct CompiledGrammar { /* items, prefilter bitmap, element table */ }

impl CompiledGrammar {
    /// True when there are no items (only "Everything Else").
    pub fn is_empty(&self) -> bool;
    /// Element table.
    pub fn elements(&self) -> &ElementTable;
    /// True when some item can leave a line in a non-`Normal` state (multi-line Delimited, or Lines with
    /// `next_lines > 0`). When false, every line starts in `Normal` and no start states are stored.
    pub fn has_multiline(&self) -> bool;
}

/// A validated, compiled format. Immutable and `Send + Sync`; shared via `Arc`.
#[derive(Debug, Clone)]
pub struct CompiledFormat { /* Arc<FileFormat>, MaskList, CompiledGrammar, compiled weights */ }

impl CompiledFormat {
    /// Validates and compiles (every regex compiled; columns/priority/tab ranges checked).
    pub fn compile(format: Arc<FileFormat>, limits: &RegexLimits) -> Result<CompiledFormat, FormatError>;
    /// The implicit "Everything Else" text format (id `None`, no grammar, Auto encoding).
    pub fn everything_else() -> CompiledFormat;
    /// Source format (`None` for Everything Else).
    pub fn format(&self) -> Option<&FileFormat>;
    /// Id (`None` for Everything Else).
    pub fn id(&self) -> Option<&FormatId>;
    /// Display name ("Everything Else" for the implicit format).
    pub fn name(&self) -> &str;
    /// Kind (Text for Everything Else).
    pub fn kind(&self) -> FormatKind;
    /// Grammar.
    pub fn grammar(&self) -> &CompiledGrammar;
    /// Text settings.
    pub fn text(&self) -> &TextSettings;
    /// Encoding setting.
    pub fn encoding(&self) -> &EncodingSetting;
    /// Conversion.
    pub fn conversion(&self) -> &Conversion;
    /// Weight (0..=4) of a raw line: the maximum priority among matching line weights (0 = none).
    pub fn line_weight(&self, line: &str) -> u8;
}

/// Tokenizer state carried from one line to the next (INV-TOK-4: it depends only on preceding lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LineState {
    /// Not inside a multi-line construct.
    #[default]
    Normal,
    /// Inside multi-line Delimited item `item`.
    InDelimited { item: u16 },
    /// Inside a Lines item with `remaining` lines still to cover.
    InLines { item: u16, remaining: u32 },
}

/// A token: byte range within one line (no EOL) and its element (INV-TOK-1..3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token {
    /// Start byte (char boundary).
    pub start: u32,
    /// End byte (exclusive, char boundary, > start).
    pub end: u32,
    /// Element (`EVERYTHING_ELSE` for uncovered text).
    pub element: ElementId,
    /// Grammar item index; `None` for everything-else runs.
    pub item: Option<u16>,
}

/// Per-line tokenizer result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOutcome {
    /// State at the start of the next line.
    pub end_state: LineState,
    /// fancy-regex runtime-limit hits on this line (treated as "no match").
    pub regex_errors: u32,
}

/// Single-threaded tokenizer with reusable scratch buffers.
pub struct Tokenizer<'g> { grammar: &'g CompiledGrammar /* + caches */ }

impl<'g> Tokenizer<'g> {
    /// New tokenizer for `grammar`.
    pub fn new(grammar: &'g CompiledGrammar) -> Tokenizer<'g>;
    /// Tokenizes one line (`line_no` is 0-based; needed for `or_line_one`). Clears and fills `out`.
    pub fn tokenize_line(&mut self, line: &str, line_no: u32, state: LineState, out: &mut Vec<Token>) -> LineOutcome;
}

/// Random access to the lines of a document (content without EOL).
pub trait LineSource: Sync {
    /// Number of lines.
    fn line_count(&self) -> u32;
    /// Line `idx`.
    fn line(&self, idx: u32) -> Cow<'_, str>;
}

/// Per-line output bytes of `tokenize_all`, stored in one byte arena per chunk plus one end offset per line
/// (no per-line heap allocation; the 1M-line bench asserts the allocation count, §6.2.4). Lines re-tokenised
/// by the speculative fix-up (§2.10.6) live in a per-chunk patch arena.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineBytes { /* chunks: Vec<{ bytes: Vec<u8>, ends: Vec<u32>, patched_prefix: u32, patch: Vec<u8>, patch_ends: Vec<u32> }> */ }

impl LineBytes {
    /// Number of lines.
    pub fn len(&self) -> u32;
    /// True when there are no lines.
    pub fn is_empty(&self) -> bool;
    /// The bytes written for `line` (O(1)). Panics never: out-of-range lines give `&[]`.
    pub fn get(&self, line: u32) -> &[u8];
    /// All outputs in line order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> + '_;
}

/// Output of `tokenize_all`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenizedLines {
    /// The callback's output bytes per line.
    pub bytes: LineBytes,
    /// Tokenizer state at the start of each line (random-access re-tokenisation for views); empty when
    /// `!grammar.has_multiline()` (every line then starts in `Normal`).
    pub start_states: Vec<LineState>,
    /// Total regex runtime-limit hits.
    pub regex_errors: u64,
}

/// Tokenizes every line and calls `f(line_no, text, tokens, out)`, where `out` is the arena of the line's
/// chunk: `f` appends the line's output bytes (hikasa-text appends the line key, §2.13.2) and must not modify
/// earlier bytes. With a pool, uses speculative chunking (§2.10.6). The result is identical to the sequential
/// run (INV-TOK-5, T2).
pub fn tokenize_all<F>(
    grammar: &CompiledGrammar,
    lines: &dyn LineSource,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
    f: F,
) -> Result<TokenizedLines, Cancelled>
where
    F: Fn(u32, &str, &[Token], &mut Vec<u8>) + Sync;
```

#### 1.4.5 Registry and presets (FF-001, FF-002, FF-004, FF-040)

```rust
/// Where a registry entry came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Embedded preset.
    BuiltIn,
    /// User file (overrides a built-in with the same id, or adds a new format).
    User { path: PathBuf },
}

/// One registry entry. Compiling happens lazily and once per entry (value-owned cache, T7).
#[derive(Debug)]
pub struct RegistryEntry {
    /// The format definition.
    pub format: Arc<FileFormat>,
    /// Globally enabled (FF-001).
    pub enabled: bool,
    /// Origin.
    pub origin: Origin,
    compiled: std::sync::OnceLock<Result<Arc<CompiledFormat>, FormatError>>,
    mask: MaskList, // compiled eagerly, case-insensitive (D-10)
}

/// Per-session enabled overrides (FF-002). Missing ids use the global flag.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnabledOverrides(pub BTreeMap<FormatId, bool>);

/// Why a format was chosen (FF-003 "Detected (C/C++ Source)").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DetectReason {
    /// The first enabled entry whose mask item matched.
    Mask { id: FormatId, mask: String },
    /// Chosen explicitly (`--format`).
    Forced { id: FormatId },
    /// No mask matched: "Everything Else".
    Fallback,
}

/// Detection result.
#[derive(Debug, Clone)]
pub struct Detection {
    /// The compiled format.
    pub format: Arc<CompiledFormat>,
    /// Why.
    pub reason: DetectReason,
}

/// Shadowing status of an entry (FF-001: shadowed entries are shown in red).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShadowStatus {
    /// Reachable by at least one name, as far as the sound test can tell.
    Visible,
    /// Every mask item is covered by earlier enabled entries (sound, may miss some cases; §2.12.3).
    Shadowed { by: Vec<FormatId> },
    /// Empty mask: manual selection only.
    ManualOnly,
    /// Disabled.
    Disabled,
}

/// Ordered list of formats (first enabled match wins).
#[derive(Debug)]
pub struct FormatRegistry { entries: Vec<RegistryEntry>, limits: RegexLimits }

impl FormatRegistry {
    /// Built-in presets in canonical order (fresh value on each call; no global cache).
    pub fn builtin() -> Result<FormatRegistry, FormatError>;
    /// From explicit entries (validates unique ids and compiles masks).
    pub fn from_entries(entries: Vec<RegistryEntry>, limits: RegexLimits) -> Result<FormatRegistry, FormatError>;
    /// Entries in order.
    pub fn entries(&self) -> &[RegistryEntry];
    /// By exact id.
    pub fn get(&self, id: &FormatId) -> Option<&RegistryEntry>;
    /// By id, then by case-insensitive name (`--format`).
    pub fn find(&self, id_or_name: &str) -> Option<&RegistryEntry>;
    /// Compiled format of an entry (cached in the entry).
    pub fn compiled(&self, id: &FormatId) -> Result<Arc<CompiledFormat>, FormatError>;
    /// Detection by file name (§2.12). Compiles the winner; a compile error is returned, not skipped.
    pub fn detect(&self, file_name: &str, overrides: &EnabledOverrides) -> Result<Detection, FormatError>;
    /// Shadowing analysis for every entry (§2.12.3).
    pub fn shadow_report(&self, overrides: &EnabledOverrides) -> Vec<(FormatId, ShadowStatus)>;
}

/// An embedded preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// File name under `presets/`.
    pub file: &'static str,
    /// TOML text (`include_str!`).
    pub toml: &'static str,
    /// Enabled by default (the only place built-in enabled flags live besides `order.toml`; D-27).
    pub enabled: bool,
}

/// All presets in canonical order.
pub const PRESETS: &[Preset] = &[ /* 01-bash.toml … 21-registry-files.toml */ ];

/// TOML schema version of format files.
pub const FORMAT_SCHEMA_VERSION: u32 = 1;

/// Parses one format file. Unknown keys become warnings (D-31); a newer schema version is an error.
pub fn parse_format(toml: &str, origin: &str) -> Result<(FileFormat, Vec<Warning>), FormatError>;
/// Canonical TOML (stable key order; round-trips: INV-FMT-1).
pub fn to_toml(format: &FileFormat) -> String;
```

#### 1.4.6 Encoding, binary detection, conversion

```rust
/// A text encoding: every WHATWG encoding (`encoding_rs`) plus UTF-32 (own decoder; FF-032 lists it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    /// An `encoding_rs` encoding.
    Whatwg(&'static encoding_rs::Encoding),
    /// UTF-32 little endian.
    Utf32Le,
    /// UTF-32 big endian.
    Utf32Be,
}

impl TextEncoding {
    /// Canonical name ("UTF-8", "windows-1252", "UTF-16LE", "UTF-32BE", …).
    pub fn name(self) -> &'static str;
    /// Lookup by label (WHATWG labels plus "utf-32le"/"utf-32be", case-insensitive).
    pub fn for_label(label: &str) -> Option<TextEncoding>;
    /// UTF-8.
    pub const UTF_8: TextEncoding;
}
impl Serialize for TextEncoding { /* name */ }
impl<'de> Deserialize<'de> for TextEncoding { /* for_label */ }

/// How the encoding was chosen (shown in the File Info header and in JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingSource {
    /// Per-pane or command-line override.
    Override,
    /// Byte-order mark.
    Bom,
    /// The format's fixed encoding.
    Fixed,
    /// BOM-less UTF-16/32 byte pattern (`utf_pattern`).
    UtfPattern,
    /// Pure ASCII content: the format's fallback label.
    FormatFallback,
    /// Pure ASCII content: the adapter's locale encoding.
    LocaleFallback,
    /// Pure ASCII content without an ASCII-compatible fallback: UTF-8.
    Ascii,
    /// The content is valid UTF-8.
    Utf8Valid,
    /// Valid UTF-8 apart from a few malformed sequences (§2.8 step 4d; added in the wave-2 review).
    Utf8Lossy,
    /// chardetng's guess.
    Detector,
}

/// A detected encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DetectedEncoding {
    /// Encoding.
    pub encoding: TextEncoding,
    /// BOM length stripped from the content (0 = no BOM).
    pub bom_len: u8,
    /// How it was chosen.
    pub source: EncodingSource,
}

/// Decoding options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Per-pane override (TC-001); wins over everything except that a matching BOM is still stripped.
    pub override_encoding: Option<TextEncoding>,
    /// "Current locale" encoding, supplied by the adapter (core never reads the locale).
    pub locale_fallback: TextEncoding,
    /// Bytes given to `chardetng`. Default 1 MiB.
    pub detect_limit: usize,
}

/// Detects the encoding (§2.8, D-42). Pure.
pub fn detect(bytes: &[u8], setting: &EncodingSetting, opts: &DecodeOptions) -> Result<DetectedEncoding, FormatError>;
/// The BOM-less UTF-32/UTF-16 byte-pattern heuristic of §2.8 step 4, on `sample` (callers pass at most the first
/// 4096 bytes). Shared by `detect` and `sniff`, so both always agree.
pub fn utf_pattern(sample: &[u8]) -> Option<TextEncoding>;
/// Decodes. Malformed sequences become U+FFFD and are counted (INV-ENC-1: never panics, total).
pub fn decode(bytes: &[u8], enc: &DetectedEncoding) -> Decoded;
/// Sniffs a BOM: (encoding, BOM length).
pub fn bom_sniff(bytes: &[u8]) -> Option<(TextEncoding, u8)>;

/// Decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Text without the BOM.
    pub text: String,
    /// Encoding used.
    pub encoding: DetectedEncoding,
    /// Number of malformed sequences replaced (0 = lossless decode).
    pub malformed: u64,
}

/// Binary-content verdict (TC-003, §2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BinaryVerdict {
    /// True when the content is classified binary.
    pub is_binary: bool,
    /// Rule that decided.
    pub reason: BinaryReason,
}

/// The rule that decided a `BinaryVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum BinaryReason {
    /// Empty content (text).
    Empty,
    /// A byte-order mark (text).
    Bom,
    /// A known binary file signature at offset 0 (binary).
    Magic { kind: MagicKind },
    /// A BOM-less UTF-16/32 pattern (`utf_pattern`; text).
    UtfPattern,
    /// A NUL byte (binary).
    NulByte { offset: u32 },
    /// Too many control bytes (binary).
    ControlRatio { permille: u16 },
    /// None of the above (text).
    Text,
}

/// Signatures recognised by `sniff` (the in-house replacement for `infer`, §0.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MagicKind { Png, Gif, Jpeg, Pdf, Zip, Gzip, Elf, SevenZip, Xz, Zstd }

/// Bytes examined by `sniff` (prefix).
pub const SNIFF_LEN: usize = 8192;
/// Classifies content as text or binary from its first `SNIFF_LEN` bytes.
pub fn sniff(bytes: &[u8]) -> BinaryVerdict;

/// Runs external programs (injected; tests use a fake; the session layer uses `StdProcessRunner`).
pub trait ProcessRunner: Send + Sync {
    /// Runs `argv` (argv[0] = program) and waits; kills the child on cancellation or timeout.
    fn run(&self, argv: &[OsString], timeout: Option<Duration>, cancel: &CancelToken) -> Result<i32, ConvertError>;
}

/// `std::process::Command` runner (no shell; stdin null; stdout/stderr discarded; polls `try_wait`
/// every 20 ms to honour cancellation and timeout).
#[derive(Debug, Clone, Copy, Default)]
pub struct StdProcessRunner;
impl ProcessRunner for StdProcessRunner { /* … */ }

/// Values substituted into command templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionVars {
    /// `%s`: path of the (temporary copy of the) source.
    pub source: PathBuf,
    /// `%t`: path the program must write.
    pub target: PathBuf,
    /// `%o`: original spec text (display only).
    pub original: String,
}

/// Splits `template` into argv and substitutes variables (pure; D-39, §2.10.8).
pub fn build_argv(template: &str, vars: &ConversionVars) -> Result<Vec<OsString>, ConvertError>;

/// Runs an external load conversion: writes `input` to `hikasa-conv.src` in a private, exclusively created
/// directory `<temp_dir>/hikasa-conv-<16 hex>` (§2.10.8), runs the program, reads `%t` (`hikasa-conv.out`).
/// Success = exit status 0 AND non-empty target (FF-030). The directory is removed in all cases.
pub fn run_external(
    conv: &ExternalConversion,
    input: &[u8],
    original: &str,
    temp_dir: &Path,
    runner: &dyn ProcessRunner,
    cancel: &CancelToken,
) -> Result<Vec<u8>, ConvertError>;
```

#### 1.4.7 Errors

```rust
/// Format errors (all carry the format id or file and a precise location).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    /// TOML syntax or type error.
    #[error("{origin}: {message}")]
    Toml { origin: String, message: String },
    /// SES-052.
    #[error("{origin}: {source}")]
    Schema { origin: String, #[source] source: SchemaError },
    /// Invalid field value.
    #[error("format {id}: {field}: {reason}")]
    Invalid { id: String, field: String, reason: String },
    /// Pattern compile error.
    #[error("format {id}: {location}: {source}")]
    Pattern { id: String, location: String, #[source] source: PatternError },
    /// Mask syntax error.
    #[error("format {id}: mask: {source}")]
    Mask { id: String, #[source] source: MaskError },
    /// Two entries with the same id.
    #[error("duplicate format id {id}")]
    DuplicateId { id: String },
    /// Unknown encoding label.
    #[error("unknown encoding {label:?}")]
    UnknownEncoding { label: String },
    /// Unknown id or name (`--format`).
    #[error("unknown format {name:?}")]
    UnknownFormat { name: String },
}

/// Pattern compile/runtime error (both engines' messages kept).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pattern {pattern:?}: {message}")]
pub struct PatternError { pub pattern: String, pub message: String, pub runtime: bool }

/// External conversion errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// Template syntax (unbalanced quote, unknown `%x`).
    #[error("command template: {0}")]
    Template(String),
    /// Program could not start.
    #[error("cannot start {program:?}: {source}")]
    Spawn { program: String, #[source] source: std::io::Error },
    /// Non-zero exit status.
    #[error("{program:?} exited with status {status}")]
    Failed { program: String, status: i32 },
    /// Exit 0 but the target is missing or empty.
    #[error("{program:?} produced no output")]
    EmptyOutput { program: String },
    /// Killed after the timeout.
    #[error("{program:?} timed out after {secs} s")]
    Timeout { program: String, secs: u32 },
    /// Temp-file I/O.
    #[error("temporary file: {0}")]
    Io(#[from] std::io::Error),
    /// Cancelled (child killed).
    #[error("operation cancelled")]
    Cancelled,
}
```

#### 1.4.8 Accepted additions and deviations (wave 2)

The review of wave 2 accepted the following public items, deviations and clarifications. They are part of the
contract: later waves may use them and must not assume the text above where an item here refines it.

**Additive API** (no signature above changes):

```rust
impl FormatId {
    pub fn new(id: impl Into<String>) -> Result<FormatId, InvalidName>;   // also TryFrom<String>, FromStr
    pub fn as_str(&self) -> &str;                                          // Display, AsRef<str>
}
impl ElementName {
    pub fn new(name: impl Into<String>) -> Result<ElementName, InvalidName>;
    pub fn as_str(&self) -> &str;
}
/// The error of `FormatId::new` / `ElementName::new` (and of their serde `try_from`).
pub struct InvalidName { pub what: &'static str, pub value: String, pub reason: &'static str }
impl FileFormat { pub fn new(id: FormatId, name: impl Into<String>, kind: FormatKind) -> FileFormat; }
impl FormatKind { pub const fn as_str(self) -> &'static str; }             // Display
impl ItemKind { pub const fn name(&self) -> &'static str; }               // "basic", "delimited", …
pub struct RawGrammarItem { /* the TOML shim of §1.4.3, public for tools */ }
pub struct GrammarItemError(pub String);                                   // its TryFrom error

impl RegistryEntry {
    pub fn new(format: Arc<FileFormat>, enabled: bool, origin: Origin) -> RegistryEntry; // mask compiled on join
    pub fn id(&self) -> &FormatId;
    pub fn mask(&self) -> Option<&MaskList>;
    pub fn is_enabled(&self, overrides: &EnabledOverrides) -> bool;
}
impl FormatRegistry {
    pub fn builtin_with_limits(limits: RegexLimits) -> Result<FormatRegistry, FormatError>;
    pub fn force(&self, id_or_name: &str) -> Result<Detection, FormatError>;     // §2.12.2, reason Forced
    pub fn everything_else(&self) -> &Arc<CompiledFormat>;
    pub fn limits(&self) -> &RegexLimits;
    pub fn into_entries(self) -> Vec<RegistryEntry>;
    /// §2.12.4 step 2. Skips (warning `user_format_skipped`) a format whose id is not its file stem, whose mask
    /// does not compile, or whose id another user format took; never fails, never loses the built-ins.
    pub fn with_user_formats(self, user: Vec<(Arc<FileFormat>, PathBuf)>) -> (FormatRegistry, Vec<Warning>);
    /// §2.12.4 steps 3–4 (unknown ids → `unknown_config_key` warnings; an id listed twice → `DuplicateId`).
    pub fn with_order(self, order: &FormatOrder) -> Result<(FormatRegistry, Vec<Warning>), FormatError>;
}
pub const ORDER_SCHEMA_VERSION: u32 = 1;
/// `formats/order.toml`: `schema_version`, then `[[format]] id = "…" enabled = true` (enabled optional).
pub struct FormatOrder { pub schema_version: u32, pub formats: Vec<OrderItem> }   // serde: `format`
pub struct OrderItem { pub id: FormatId, pub enabled: Option<bool> }
impl FormatOrder {
    pub fn from_registry(registry: &FormatRegistry) -> FormatOrder;
    pub fn parse(toml_text: &str, origin: &str) -> Result<(FormatOrder, Vec<Warning>), FormatError>;
    pub fn to_toml(&self) -> String;
}
impl Preset { pub fn origin(&self) -> String; }                            // "built-in:<file>"

pub const EVERYTHING_ELSE: &str = "Everything Else";
impl ElementTable { pub fn iter(&self) -> impl Iterator<Item = (ElementId, &str)> + '_; pub fn is_empty(&self) -> bool; }
impl CompiledGrammar { pub fn item_count(&self) -> usize; }
impl CompiledFormat {
    pub fn mask(&self) -> Option<&MaskList>;
    pub fn has_line_weights(&self) -> bool;
    /// `line_weight` plus the runtime-limit hits of the weight regexes (each counts as "not found"; §2.10.7).
    pub fn line_weight_with_errors(&self, line: &str) -> (u8, u32);
}
impl Pattern {
    pub fn source(&self) -> &str;
    pub fn case_insensitive(&self) -> bool;
    pub fn is_accelerated(&self) -> bool;            // the exact ASCII accelerator of §2.10.5 applies
}

impl Token { pub fn text<'l>(&self, line: &'l str) -> &'l str; }
/// INV-TOK-1..3 for one line's tokens.
pub fn check_tokens(line: &str, tokens: &[Token]) -> Result<(), InvariantViolation>;
impl<'g> Tokenizer<'g> {
    pub fn grammar(&self) -> &'g CompiledGrammar;
    /// `tokenize_line` that also observes `cancel` inside the line (every 64 KiB of scanned text and before
    /// every regex engine call); on cancellation `out` is cleared. `tokenize_all` uses it.
    pub fn tokenize_line_cancellable(&mut self, line: &str, line_no: u32, state: LineState, out: &mut Vec<Token>,
                                     cancel: &CancelToken) -> Result<LineOutcome, Cancelled>;
    #[doc(hidden)] pub fn reference(grammar: &'g CompiledGrammar) -> Tokenizer<'g>;   // tests: no fast paths
}
#[doc(hidden)] pub fn tokenize_all_with_chunk<F>(/* tokenize_all + chunk_lines: u32 */);  // P-TOK-5
impl LineSource for [&str] / [String] / Vec<&str> / Vec<String>;
impl LineBytes { pub fn patched_lines(&self) -> u32; }
impl TokenizedLines { pub fn check_invariants(&self) -> Result<(), InvariantViolation>; }  // T4
/// The text dump of `hikasa format tokens`: `L<line>:<start>-<end>\t<element>\t<escaped text>\n` per token.
pub fn write_token_dump(out: &mut String, line_number: u32, line: &str, tokens: &[Token], elements: &ElementTable);
pub fn escape_token_text(text: &str) -> String;   // `\` `\t` `\r` `\n`, other controls `\u{XX}`

impl TextEncoding {
    pub const UTF_16LE: TextEncoding;
    pub const UTF_16BE: TextEncoding;
    pub fn is_ascii_compatible(self) -> bool;
    pub fn bom(self) -> &'static [u8];               // empty for encodings without one
}
impl Display for TextEncoding;                       // name()
impl Default for DecodeOptions;                      // no override, UTF-8 locale, 1 MiB detect limit
pub const UTF_PATTERN_SAMPLE: usize = 4096;
/// Encodes without a BOM; UTF-16/32 by Hikasa, the rest by encoding_rs (unmappable → numeric references).
pub fn encode(text: &str, enc: TextEncoding) -> (Vec<u8>, bool);
pub enum EncodingSource { /* …, */ Utf8Lossy /* , … */ }   // §2.8 step 4d, serde `utf8_lossy`
pub const MAGIC: &[(MagicKind, &[u8])];
pub fn magic(bytes: &[u8]) -> Option<MagicKind>;
/// The Sorted conversion (D-38) as a pure text function.
pub fn sort_lines(text: &str) -> String;
impl From<Cancelled> for ConvertError;               // ConvertError::Cancelled
#[derive(Clone)] pub enum FormatError { /* … */ }    // the registry hands out a cached compile error
```

**Deviations and clarifications:**

1. `with_user_formats` returns `(FormatRegistry, Vec<Warning>)`; the wave-2 implementation returned
   `Result<FormatRegistry, FormatError>` and failed (consuming the registry) on one bad user mask. This is the
   only signature change of the review; no later wave used the old one.
2. `EncodingSource::Utf8Lossy` is a new variant (§2.8 step 4d, D-42 as amended). Downstream matches over
   `EncodingSource` must handle it.
3. `FormatError::Schema` classifies as `unsupported`, as `SchemaError` itself (§1.1.12); §5.5 rule 4 says so.
   A `schema_version` that does not fit a `u32` is `Schema` (reported as 4294967295), not a type error.
4. `ElementName` also rejects control characters (names appear in the token dump and in JSON); the name
   "Everything Else" is reserved (a grammar item or `[elements]` key with it is `FormatError::Invalid`).
5. `TextEncoding::for_label`: labels that WHATWG maps to the replacement encoding (ISO-2022-KR, HZ-GB-2312, …)
   give `None`; "utf-32", "utf32" and "utf32le" mean UTF-32LE (as WHATWG's "utf-16" means UTF-16LE), "utf32be"
   means UTF-32BE.
6. Detection and `--format` matching use the last path component of the name (anything up to the last `/` or
   `\` is ignored, D-10). A mask with exclude items only can never match: `ShadowStatus::ManualOnly`.
7. `LineBytes` equality compares the logical content (the bytes of every line), not the arena layout, so a
   patched parallel run equals the sequential one (INV-TOK-5).
8. `tokenize_all` reports no progress (§1.4.4); hikasa-text reports the `tokenizing` phase from its callback
   (§5.4 as amended).
9. Field boundaries for Columns items (§2.10.2 as amended): tokens of other items never cross the first column
   of a Columns item. Regex items are defined equivalently by anchored matches (§2.10.2), which the reference
   tokenizer uses at every position.
10. `FileFormat`'s serde form is the canonical TOML one: empty `elements`, `grammar` and `line_weight` and the
    default fields of grammar items are omitted (INV-FMT-1 holds). JSON shapes are owned by DTOs (T5): the
    `format` member of `hikasa.format_show.v1` (§4.3.4) is built by the session DTO with every section present
    (`elements: {}`, `grammar: []`, `line_weight: []`, `table: null`, every grammar-item field of its kind),
    so §4.1 rule 3 holds there.
11. External conversions run in a private directory (§2.10.8): the names of `%s` and `%t` changed from
    `<temp_dir>/hikasa-conv-<n>.{src,out}` to `<temp_dir>/hikasa-conv-<16 hex>/hikasa-conv.{src,out}`. The name
    uses a per-process counter (a `static AtomicU64`): an accepted T7 exception, because it only names
    temporary directories and never influences a result. Descendants of a killed program are not killed (no
    approved process-group API, §0.7).
12. The global-pool trap of this crate (§5.1) builds the rayon global pool with recognisable thread names and
    records every callback that runs on one of them, instead of a panicking `start_handler`: rayon starts the
    global pool's threads when the pool is built, so a start handler cannot see later implicit use. Every
    `tokenize_all` work item calls the callback, so any implicit use is seen.
13. The ASCII accelerator, the first-char prefilter, the keyword hash set and the next-match cache are the
    performance design of §2.10.5; they never change a result (P-TOK-REF compares them with the reference
    tokenizer, including the regex error counts).
14. `LineWeight` regexes that hit the runtime limit are counted through `line_weight_with_errors`;
    `line_weight` itself keeps the §1.4.4 signature.

### 1.5 hikasa-text

**Purpose.** The text document model (TC-001…TC-003), text rules (TC-010…TC-022), the comparison engine
(tokenize → keys → align → rows → sections → stats), the thumbnail and display filters (TC-032, TC-035, TC-040),
a progressive observer for GUIs (ARCH-004), and the rules-based content comparator that `hikasa-folder`
receives by injection (FC-025). The result model gives O(1) random access by row index, for a virtualised GUI
and for reports.
**Uses.** hikasa-core, hikasa-diff, hikasa-formats, hikasa-vfs, ropey, serde, thiserror, unicode-segmentation,
memchr, rayon, toml, bitflags.

Re-exports (§0.4 rule 8), so that `hikasa-report` and `hikasa-cli` need no extra manifest edges:

```rust
pub use hikasa_formats::{
    BinaryReason, BinaryVerdict, CompiledFormat, DetectReason, DetectedEncoding, ElementId, ElementTable,
    EncodingSource, FormatId, FormatKind, LineState, MagicKind, RegexLimits, TextEncoding, Token,
};
pub use hikasa_diff::{Algorithm, InlineDiff, Run, RunKind};
```

#### 1.5.1 Module tree

```
core/hikasa-text/src/
├── lib.rs           re-exports
├── document/
│   ├── mod.rs       TextDocument, LineIdx, LineEnding, EolSummary, EolStyle, DocOrigin, MAX_LINES
│   ├── lines.rs     LineTable: CR/LF/CRLF splitting (memchr), characters-per-line wrapping (TC-003), D-43 limit
│   └── load.rs      LoadOptions, TextDocument::from_bytes (pure), load_document (via Vfs), Sorted conversion
├── rules.rs         TextRules, ImportanceRules, AlignmentRules, AlgorithmChoice, TextPattern, UnimportantItem,
│                    TextRulesPatch
├── classify.rs      CharClass, ClassifiedSpan, line keys (atoms, §2.13), identity fast path, trivial lines
├── compare/
│   ├── mod.rs       CompareInput, CompareObserver, PartialRows, compare(), TextComparison
│   ├── rows.rs      RowIdx, Row, RowState, Importance (packed storage, 12 bytes/row)
│   ├── sections.rs  SectionId, Section, SectionKind (§2.17)
│   ├── inline.rs    ClassifiedInline, InlineRange (TC-014)
│   ├── thumb.rs     ThumbCell, thumbnail (TC-032)
│   ├── nav.rs       Direction, next/previous section, row_for_line
│   ├── keys.rs      (wave 3, private) per-side key arenas, line weights, the context-free check of §2.19 step 5
│   ├── score.rs     (wave 3, private) the closeness scorer of §2.15.3 and its work budget
│   └── check.rs     (private) TextComparison::check_invariants
├── stats.rs         TextStats, ChangeCounts
├── view.rs          TextDisplayFilter, TextToggles, ViewOptions, FilteredView, ViewItem (§2.18)
├── quick.rs         RulesComparator, quick_compare_bytes, DEFAULT_RULES_MAX_BYTES (§2.19)
└── error.rs         TextError
```

#### 1.5.2 Document model

```rust
/// 0-based line index within one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineIdx(pub u32);

/// Most lines a document may have (D-43); equal to `hikasa_diff::MAX_SEQUENCE_LEN`.
pub const MAX_LINES: u32 = hikasa_diff::MAX_SEQUENCE_LEN;

/// Line terminator of one line (TC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// "\n".
    Lf,
    /// "\r\n".
    CrLf,
    /// "\r" not followed by "\n".
    Cr,
    /// Last line without a terminator.
    None,
    /// Artificial break from the characters-per-line limit (removed on save; TC-003).
    Artificial,
}

/// Counts of terminators (`Artificial` breaks are not counted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EolSummary { pub lf: u64, pub crlf: u64, pub cr: u64, pub none: u64 }

/// Summarised style ("MIX" when inconsistent, TC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EolStyle { Lf, CrLf, Cr, Mixed, None }

impl EolSummary {
    /// The style: the single kind used by all terminated lines; `Mixed` if there are several kinds;
    /// `None` if there are no terminated lines.
    pub fn style(&self) -> EolStyle;
    /// The most frequent kind (ties: Lf > CrLf > Cr), used by the Sorted conversion.
    pub fn dominant(&self) -> LineEnding;
}

/// Where a document came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocOrigin {
    /// Spec (None for in-memory test documents).
    pub spec: Option<Spec>,
    /// Modification time of the source.
    pub mtime: Option<FileTime>,
    /// Source size in bytes (before conversion).
    pub size: u64,
}

/// Document load options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    /// Encoding detection and decoding options (incl. the per-pane override, TC-001).
    pub decode: DecodeOptions,
    /// Overrides the format's characters-per-line limit (`Some(0)` disables the limit).
    pub chars_per_line_limit: Option<u32>,
    /// Maximum source size. Default and hard cap: 4 GiB − 1 (line tables store u32 byte offsets).
    pub max_bytes: u64,
}

/// An immutable decoded text document (TC-001). `Send + Sync`; shared via `Arc`.
/// Storage: a `ropey::Rope` plus Hikasa's own line table. Ropey's built-in line indexing is NOT used
/// because its default features also break lines at Unicode separators (VT, FF, NEL, LS, PS).
/// Hikasa lines end only at CR, LF and CRLF (INV-DOC-1).
#[derive(Debug, Clone)]
pub struct TextDocument { /* rope, line table, DetectedEncoding, malformed, EolSummary, BinaryVerdict, converted, origin */ }

impl TextDocument {
    /// Builds a document from raw bytes. Pure: binary sniff → Ctrl-Z → detect/decode → line split →
    /// Sorted conversion (when the format says so) → CPL wrapping. Never fails on content; it fails only on
    /// `max_bytes` (`TooLarge`), on more than `MAX_LINES` lines (`TooManyLines`, D-43) or on an unknown
    /// encoding label.
    pub fn from_bytes(bytes: &[u8], format: &CompiledFormat, opts: &LoadOptions, origin: DocOrigin)
        -> Result<TextDocument, TextError>;
    /// UTF-8 text with no detection (tests and stdin pipelines that are already text).
    pub fn from_text(text: &str) -> TextDocument;
    /// Number of lines (`""` has 0 lines; `"\n"` has 1).
    pub fn line_count(&self) -> u32;
    /// Line content without its terminator (borrowed when the rope chunk is contiguous).
    pub fn line(&self, idx: LineIdx) -> Cow<'_, str>;
    /// Terminator of `idx`.
    pub fn line_ending(&self, idx: LineIdx) -> LineEnding;
    /// 1-based physical line number in the source (differs from idx+1 only with CPL wrapping).
    pub fn physical_line(&self, idx: LineIdx) -> u32;
    /// Encoding used.
    pub fn encoding(&self) -> &DetectedEncoding;
    /// Malformed byte sequences replaced while decoding (0 = lossless).
    pub fn malformed(&self) -> u64;
    /// Terminator counts.
    pub fn eol(&self) -> &EolSummary;
    /// Binary verdict from the raw bytes (TC-003).
    pub fn binary(&self) -> BinaryVerdict;
    /// True when a conversion produced the text (FF-031: read-only).
    pub fn converted(&self) -> bool;
    /// Origin.
    pub fn origin(&self) -> &DocOrigin;
    /// The rope (GUI rendering).
    pub fn rope(&self) -> &ropey::Rope;
}
impl hikasa_formats::LineSource for TextDocument { /* line_count, line */ }

/// Loads `spec` through `vfs`: stat, size check, read (cancellation checked per 1 MiB), optional External
/// conversion (byte stage, run in `temp_dir` with `runner`), then `from_bytes`. The only I/O entry point of the
/// crate. Returns the raw bytes too, so the caller can compute `bytes_identical` and then drop them.
pub fn load_document(
    vfs: &dyn Vfs,
    spec: &Spec,
    format: &CompiledFormat,
    opts: &LoadOptions,
    external: Option<(&Path, &dyn ProcessRunner)>,
    ctl: &Ctl<'_>,
) -> Result<(TextDocument, Vec<u8> /* raw bytes */), TextError>;
```

#### 1.5.3 Rules (persisted TOML, `hikasa text compare --rules-file <file>`)

```rust
/// Schema version of rules files.
pub const TEXT_RULES_SCHEMA_VERSION: u32 = 1;

/// Session rules of a Text Compare (TC-010…TC-022). `Default` = D-04.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextRules {
    /// Schema version (SES-052).
    pub schema_version: u32,
    /// Importance tab.
    pub importance: ImportanceRules,
    /// Alignment tab.
    pub alignment: AlignmentRules,
}

/// Importance rules (TC-020…TC-022). Defaults (D-04) give an exact comparison apart from line endings,
/// encoding and BOM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportanceRules {
    /// Element name → important. Overrides the format's `ElementInfo::important` for that name; names not
    /// listed use the format's default. Default: empty.
    pub elements: BTreeMap<String, bool>,
    /// Leading whitespace important. Default true.
    pub leading_whitespace: bool,
    /// Embedded whitespace important. Default true.
    pub embedded_whitespace: bool,
    /// Trailing whitespace important. Default true.
    pub trailing_whitespace: bool,
    /// Text not covered by grammar items is important. Default true.
    pub everything_else: bool,
    /// Case changes in everything-else text are important. Default true.
    pub everything_else_case_sensitive: bool,
    /// "Unimportant text" patterns (TC-020), applied before everything else. Default: none.
    pub unimportant: Vec<TextPattern>,
    /// TC-021. Default true.
    pub orphans_always_important: bool,
    /// TC-022. Default false.
    pub compare_line_endings: bool,
}

impl ImportanceRules {
    /// Marks every item unimportant: whitespace classes and everything-else set their flag to false, `Case`
    /// sets `everything_else_case_sensitive = false`, `Element(n)` sets `elements[n] = false`.
    pub fn apply_unimportant(&mut self, items: &[UnimportantItem]);
}

/// A plain or regex pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextPattern {
    /// Pattern text.
    pub pattern: String,
    /// Regex (engine chosen per §2.11).
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive. Default true.
    #[serde(default = "yes")]
    pub match_case: bool,
}

/// One word of the `--unimportant` vocabulary, shared by the CLI and (M2) scripts (T11).
/// Serialised as its word (`"leading-ws"`, …, or `"element:<Name>"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum UnimportantItem {
    /// `leading-ws`.
    LeadingWhitespace,
    /// `embedded-ws`.
    EmbeddedWhitespace,
    /// `trailing-ws`.
    TrailingWhitespace,
    /// `whitespace` = the three classes above.
    Whitespace,
    /// `case` = everything-else text is compared case-insensitively.
    Case,
    /// `everything-else`.
    EverythingElse,
    /// Any other word, or `element:<Name>` (needed only when the name equals a vocabulary word): a grammar
    /// element name (exact, case-sensitive).
    Element(String),
}

impl UnimportantItem {
    /// Parses a comma-separated list. Items are trimmed; empty items are ignored; the fixed words match
    /// ASCII-case-insensitively. An item that is not a valid `ElementName` is `TextError::Rules`.
    pub fn parse_list(list: &str) -> Result<Vec<UnimportantItem>, TextError>;
    /// The canonical word.
    pub fn as_str(&self) -> Cow<'_, str>;
}

/// Algorithm choice; `Auto` per D-36.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmChoice { #[default] Auto, Unaligned, Standard, Myers, Patience, Histogram }

/// Alignment rules (TC-010, TC-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignmentRules {
    /// Engine. Default Auto.
    pub algorithm: AlgorithmChoice,
    /// Skew tolerance (lines). Default 300.
    pub skew_tolerance: u32,
    /// Closeness matching (D-17). Default true.
    pub closeness: bool,
    /// Closeness threshold (per mille, 0..=1000). Default 500.
    pub closeness_threshold_permille: u16,
    /// Never align differences. Default false.
    pub never_align_differences: bool,
}

/// Overrides from CLI flags or IPC options (`None` = keep; vectors are appended). Applied after the
/// `--rules-file` TOML (§3.5). Each field is independent, so a lone `--skew 100` keeps `AlgorithmChoice::Auto`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TextRulesPatch {
    pub algorithm: Option<AlgorithmChoice>,
    pub skew_tolerance: Option<u32>,
    pub closeness: Option<bool>,
    pub closeness_threshold_permille: Option<u16>,
    pub never_align_differences: Option<bool>,
    pub compare_line_endings: Option<bool>,
    pub orphans_always_important: Option<bool>,
    /// `--unimportant <list>` items, applied with `ImportanceRules::apply_unimportant` after the fields above.
    pub unimportant: Vec<UnimportantItem>,
    /// `--unimportant-text <text>` (plain) and `--unimportant-regex <re>` patterns, appended in order.
    pub unimportant_text: Vec<TextPattern>,
}

impl TextRules {
    /// Parses a rules file (unknown keys → warnings; newer schema → error).
    pub fn from_toml(text: &str) -> Result<(TextRules, Vec<Warning>), TextError>;
    /// Canonical TOML (round-trips: INV-RULES-1).
    pub fn to_toml(&self) -> String;
    /// Applies a patch.
    pub fn apply(&mut self, patch: &TextRulesPatch);
    /// Validates value ranges and compiles `unimportant` patterns.
    pub fn validate(&self, limits: &RegexLimits) -> Result<(), TextError>;
    /// Effective importance of element `id` of `format`: `importance.elements[name]` if present, else the
    /// format's `ElementInfo::important`; `ElementId::EVERYTHING_ELSE` → `importance.everything_else`.
    pub fn element_important(&self, format: &CompiledFormat, id: ElementId) -> bool;
    /// The identity fast-path predicate (D-53, §2.13.4): no unimportant patterns, every whitespace class and
    /// everything-else important and case-sensitive, and every element of `format` important and
    /// case-sensitive.
    pub fn is_identity(&self, format: &CompiledFormat) -> bool;
}
```

#### 1.5.4 Comparison result model

```rust
/// Inputs of a comparison. One effective format applies to both sides (D-15).
#[derive(Debug, Clone)]
pub struct CompareInput {
    /// Left document.
    pub left: Arc<TextDocument>,
    /// Right document.
    pub right: Arc<TextDocument>,
    /// Effective format.
    pub format: Arc<CompiledFormat>,
    /// Rules.
    pub rules: TextRules,
    /// True when the raw source bytes were identical (enables the `BinarySame` verdict).
    pub bytes_identical: bool,
    /// Disables the identity fast path (property tests and benches compare both paths; D-53). Default false.
    pub force_tokenize: bool,
}

/// Progressive observer (ARCH-004; the GUI's first screen, ARCH-GUI-003). Called on the thread that runs
/// `compare` (and `Service::text_compare`); implementations must return quickly.
pub trait CompareObserver: Send {
    /// Both documents are loaded (called by `Service::text_compare` before `compare` starts), so a GUI can
    /// render unaligned panes at once.
    fn loaded(&mut self, _left: &Arc<TextDocument>, _right: &Arc<TextDocument>) {}
    /// A phase started (`Tokenizing`, `Aligning`, `Classifying`).
    fn phase(&mut self, _phase: Phase) {}
    /// A confirmed prefix of the alignment (forwarded from `hikasa_diff::RunSink`): `rows.runs` extends the runs
    /// of the previous call; every row covering left lines `< left_done` and right lines `< right_done` is final
    /// (not yet classified). Standard alignment reports progressively; other engines report the common prefix
    /// once and the rest at the end.
    fn partial(&mut self, _rows: &PartialRows<'_>) {}
}

/// Partial result passed to `CompareObserver::partial`.
#[derive(Debug, Clone, Copy)]
pub struct PartialRows<'a> {
    /// New runs, in document order.
    pub runs: &'a [Run],
    /// Left lines whose rows are final.
    pub left_done: u32,
    /// Right lines whose rows are final.
    pub right_done: u32,
}

/// Compares two documents (§2.13–§2.17). Pure apart from `pool` usage, and deterministic for any pool (T2).
/// The observer never changes the result.
pub fn compare(
    input: CompareInput,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    observer: Option<&mut dyn CompareObserver>,
) -> Result<TextComparison, TextError>;

/// Row index (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowIdx(pub u32);

/// Row state (TC-037 vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    /// Both lines present and identical after rules.
    Same,
    /// Both present, different (importance says whether it matters).
    Changed,
    /// Left orphan.
    LeftOnly,
    /// Right orphan.
    RightOnly,
}

/// Importance of a row. INV-ROW-1: `state == Same` ⇔ `importance == None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance { None, Unimportant, Important }

/// One alignment row (O(1) random access).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Row {
    /// Left line (None for RightOnly).
    pub left: Option<LineIdx>,
    /// Right line (None for LeftOnly).
    pub right: Option<LineIdx>,
    /// State.
    pub state: RowState,
    /// Importance.
    pub importance: Importance,
}

/// Section id: ordinal in row order (stable for identical inputs; ARCH-050).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SectionId(pub u32);

/// Shape of a section (normal-diff a/d/c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind { Add, Delete, Change }

/// A maximal run of non-Same rows (or a single row for `lines_independent` formats).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Section {
    /// Id.
    pub id: SectionId,
    /// Rows `[start, end)`.
    pub rows: Range<u32>,
    /// Left lines covered `[start, end)` (empty for Add; start = insertion point).
    pub left_lines: Range<u32>,
    /// Right lines covered (empty for Delete).
    pub right_lines: Range<u32>,
    /// Kind.
    pub kind: SectionKind,
    /// Important if any row is important, else Unimportant.
    pub importance: Importance,
}

/// One inline range, classified (TC-014).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InlineRange {
    /// Byte range in the line text.
    pub range: Range<u32>,
    /// Important (red) or unimportant (blue).
    pub important: bool,
}

/// Classified inline differences of a two-sided row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClassifiedInline {
    /// Left ranges.
    pub left: Vec<InlineRange>,
    /// Right ranges.
    pub right: Vec<InlineRange>,
    /// The line exceeded the inline limit.
    pub whole_line: bool,
}

/// A coloured span of a line (syntax element + importance), for views.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassifiedSpan {
    /// Byte range.
    pub range: Range<u32>,
    /// Element.
    pub element: ElementId,
    /// Char class used for importance.
    pub class: CharClass,
    /// Importance under the rules.
    pub important: bool,
}

/// Char classes (§2.13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharClass { UnimportantPattern, Element, LeadingWhitespace, EmbeddedWhitespace, TrailingWhitespace, Other }

/// Search direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction { Forward, Backward }

bitflags::bitflags! {
    /// One thumbnail bucket (TC-032): which row classes occur among the bucket's rows.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ThumbCell: u8 {
        const SAME = 1; const UNIMPORTANT = 2; const IMPORTANT = 4; const LEFT_ONLY = 8; const RIGHT_ONLY = 16;
    }
}

/// Immutable comparison result. `Send + Sync`; share via `Arc`. INV-TC-1..8 (Appendix B).
#[derive(Debug)]
pub struct TextComparison { /* docs, format, rules, packed rows, sections, per-line keys/start states (lazy),
                               stats, warnings */ }

impl TextComparison {
    /// Number of rows.
    pub fn row_count(&self) -> u32;
    /// Row `idx` (O(1), < 200 ns: gate G5). Panics never: out-of-range returns None.
    pub fn row(&self, idx: RowIdx) -> Option<Row>;
    /// Rows in a range (clamped).
    pub fn rows(&self, range: Range<u32>) -> impl Iterator<Item = Row> + '_;
    /// Document of `side` (Left or Right; `Center`/`Output` return the left document in M1).
    pub fn document(&self, side: Side) -> &Arc<TextDocument>;
    /// Effective format.
    pub fn format(&self) -> &Arc<CompiledFormat>;
    /// Rules used.
    pub fn rules(&self) -> &TextRules;
    /// Algorithm actually used (after resolving Auto).
    pub fn algorithm(&self) -> Algorithm;
    /// Classified inline differences for a two-sided non-Same row (lazy, pure; None otherwise; p99 < 50 µs:
    /// gate G5).
    pub fn inline(&self, idx: RowIdx) -> Option<ClassifiedInline>;
    /// Computes the per-line tokenizer start states of `side` when the identity fast path skipped
    /// tokenisation and the grammar has multi-line items (one sequential pass, cached). GUIs call it on a worker
    /// before the first `spans` call; `spans` calls it implicitly otherwise.
    pub fn prepare_spans(&self, side: Side, cancel: &CancelToken) -> Result<(), Cancelled>;
    /// Spans of a line (re-tokenises from the stored start state; O(line length) once prepared).
    pub fn spans(&self, side: Side, line: LineIdx) -> Vec<ClassifiedSpan>;
    /// Sections in row order.
    pub fn sections(&self) -> &[Section];
    /// Section containing a row.
    pub fn section_at(&self, idx: RowIdx) -> Option<SectionId>;
    /// Next/previous section from `from` (exclusive), skipping Unimportant ones when `ignore_unimportant`
    /// (TC-050 Next Difference Section).
    pub fn next_section(&self, from: RowIdx, dir: Direction, ignore_unimportant: bool) -> Option<SectionId>;
    /// Row showing `line` of `side` (Go To line).
    pub fn row_for_line(&self, side: Side, line: LineIdx) -> Option<RowIdx>;
    /// Thumbnail (TC-032): `buckets` cells covering the rows evenly (bucket `b` covers rows
    /// `[b·rows/buckets, (b+1)·rows/buckets)`), built from the sections in O(S + buckets).
    /// With `ignore_unimportant`, unimportant sections count as SAME.
    /// (Wave 3, §1.5.7: built from the class runs in O(runs + buckets); unimportant *rows* count as SAME;
    /// buckets that cover no row are empty.)
    pub fn thumbnail(&self, buckets: u32, ignore_unimportant: bool) -> Vec<ThumbCell>;
    /// Statistics (§2.17.3).
    pub fn stats(&self) -> &TextStats;
    /// Verdict (§2.17.5); `ignore_unimportant` maps UnimportantOnly to RulesSame.
    pub fn verdict(&self, ignore_unimportant: bool) -> ContentResult;
    /// Filtered view (§2.18).
    pub fn view(&self, opts: &ViewOptions) -> FilteredView;
    /// Warnings (binary content, lossy decode, format mismatch, budgets, closeness ignored …).
    /// (Wave 3, §1.5.7: `format_mismatch` is emitted by the caller that chooses the format, not by `compare`.)
    pub fn warnings(&self) -> &[Warning];
    /// Re-checks INV-TC-1..8.
    pub fn check_invariants(&self) -> Result<(), InvariantViolation>;
}

/// Text statistics (RPT text "Statistics" + "Summary").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextStats {
    /// Rows.
    pub rows: u64,
    /// Same rows.
    pub same_rows: u64,
    /// Changed rows (both sides present).
    pub changed_rows: u64,
    /// Left-only rows.
    pub left_only_rows: u64,
    /// Right-only rows.
    pub right_only_rows: u64,
    /// Rows with importance Important.
    pub important_rows: u64,
    /// Rows with importance Unimportant.
    pub unimportant_rows: u64,
    /// Section-based counts for important sections (D-18).
    pub important: ChangeCounts,
    /// Section-based counts for unimportant sections.
    pub unimportant: ChangeCounts,
    /// Sections.
    pub sections: u64,
    /// Important sections.
    pub important_sections: u64,
    /// Left document lines.
    pub left_lines: u64,
    /// Right document lines.
    pub right_lines: u64,
}

/// Added/deleted/changed/original line counts (D-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeCounts { pub added: u64, pub deleted: u64, pub changed: u64, pub original: u64 }
```

#### 1.5.5 Views (TC-040, TC-035, RPT-010)

```rust
bitflags::bitflags! {
    /// Atomic text display categories (toggle mode).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TextToggles: u8 { const SAME = 1; const LEFT_ORPHANS = 2; const RIGHT_ORPHANS = 4; const DIFFERENCES = 8; }
}

/// Display filter (TC-040).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TextDisplayFilter {
    /// Show All.
    All,
    /// Show Differences.
    Differences,
    /// Show Same.
    Same,
    /// Show None.
    None,
    /// Show Context (differences ± `lines`).
    Context { lines: u32 },
    /// Toggle mode (`TextToggles` bits).
    Toggles { bits: u8 },
}

/// View options.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ViewOptions {
    /// Filter.
    pub filter: TextDisplayFilter,
    /// TC-024: unimportant counts as same for display, navigation, counts and reports.
    pub ignore_unimportant: bool,
    /// Gap bars the user expanded (TC-035), as row ranges: rows inside them are visible whatever the filter.
    /// Sorted or not, overlapping or not (normalised on use). Default empty; reports leave it empty.
    #[serde(default)]
    pub expanded: Vec<Range<u32>>,
}

/// A visible item: a row, or a gap of hidden rows ("N FILTERED LINES" / "N lines hidden").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewItem {
    /// A visible row.
    Row { row: RowIdx },
    /// Hidden rows `[start, end)`.
    Gap { rows: Range<u32> },
}

/// Random-access filtered view: O(log k) `get` over k segments (INV-VIEW-1..3; < 200 ns: gate G5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredView { /* segments + prefix sums */ }

impl FilteredView {
    /// Number of items (rows + gaps).
    pub fn len(&self) -> u32;
    /// True when no items.
    pub fn is_empty(&self) -> bool;
    /// Item at visible position `pos`.
    pub fn get(&self, pos: u32) -> Option<ViewItem>;
    /// All items in order.
    pub fn iter(&self) -> impl Iterator<Item = ViewItem> + '_;
    /// Visible position of `row`: `Ok(pos)` when visible, `Err(pos)` of the gap item that hides it.
    pub fn position_of(&self, row: RowIdx) -> Result<u32, u32>;
    /// Number of hidden rows.
    pub fn hidden_rows(&self) -> u64;
}
```

#### 1.5.6 Rules-based content comparator (FC-025, TC-016)

```rust
/// Default per-file limit of `RulesComparator` (D-25).
pub const DEFAULT_RULES_MAX_BYTES: u64 = 128 * 1024 * 1024;

/// Rules-based `ContentComparator` (injected into hikasa-folder by the session layer).
#[derive(Debug, Clone)]
pub struct RulesComparator { /* Arc<FormatRegistry>, EnabledOverrides, forced format, TextRules, LoadOptions, max_bytes */ }

impl RulesComparator {
    /// Validates the rules up front (T9).
    pub fn new(registry: Arc<FormatRegistry>, rules: TextRules, load: LoadOptions) -> Result<RulesComparator, TextError>;
    /// Per-session enabled overrides (FF-002).
    pub fn with_overrides(self, overrides: EnabledOverrides) -> RulesComparator;
    /// Forces one format for every pair.
    pub fn with_forced_format(self, format: Arc<CompiledFormat>) -> RulesComparator;
    /// Per-file limit (default `DEFAULT_RULES_MAX_BYTES`); `size_limit()` returns it. Larger files are
    /// compared by bytes with warning `rules_size_limit` (D-25).
    pub fn with_max_bytes(self, max: u64) -> RulesComparator;
}
impl ContentComparator for RulesComparator {
    /* method() == Rules; size_limit() == Some(max_bytes); algorithm §2.19; reads with
       `ContentSource::read_all(max_bytes, ctl.cancel)`; the effective format follows D-15 per pair */
}

/// The shared quick-compare core (used by `RulesComparator` and by `hikasa quick --rules`). Each side is
/// `(bytes, name for format detection)`; `format` is the effective format already chosen by the caller (D-15).
pub fn quick_compare_bytes(
    left: (&[u8], &str),
    right: (&[u8], &str),
    format: &CompiledFormat,
    rules: &TextRules,
    load: &LoadOptions,
    ctl: &Ctl<'_>,
) -> Result<ContentOutcome, Cancelled>;

/// Text errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TextError {
    /// The input could not be read (`Side` implements `Display`, §1.1.2).
    #[error("{side} input: {source}")]
    Load { side: Side, #[source] source: VfsError },
    /// Document over `max_bytes`.
    #[error("{spec}: larger than {limit} bytes")]
    TooLarge { spec: String, limit: u64 },
    /// Document with more than `MAX_LINES` lines (D-43).
    #[error("{spec}: {lines} lines exceed the limit of {limit}")]
    TooManyLines { spec: String, lines: u64, limit: u64 },
    /// Format problem (compile, unknown encoding).
    #[error(transparent)]
    Format(#[from] FormatError),
    /// External conversion failed.
    #[error(transparent)]
    Convert(#[from] ConvertError),
    /// Invalid rules (range, pattern, vocabulary word).
    #[error("rules: {0}")]
    Rules(String),
    /// Alignment error.
    #[error(transparent)]
    Diff(#[from] DiffError),
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
}
```

#### 1.5.7 Accepted additions and deviations (wave 3)

The wave-3 review (adversarial and conformance findings, maintainer decisions 1–9) accepted the following
public items, deviations and amendments of hikasa-text. They are part of the contract: hikasa-report and
hikasa-session (wave 4) build on them and must not assume the text above where an item here refines it. No
signature of §1.5.1–§1.5.6 changes; every change is additive. Amended normative text: D-36, D-53 (§0.6), §0.5,
§0.8 and §5.7 text rows, §2.13.2–§2.13.4, §2.15.2–§2.15.3, §2.16, §2.17.1, §2.19, §6.2.5, Appendix B.

**Additive API** (complete list beyond §1.5.1–§1.5.6):

```rust
// Modules: classify, compare, document, error, quick, rules, stats, view are public; every item is also
// re-exported at the crate root.
// document
pub const MAX_DOCUMENT_BYTES: u64 = u32::MAX as u64;            // default and hard cap of LoadOptions::max_bytes
impl Default for LoadOptions { /* default decoding, the format's CPL limit, MAX_DOCUMENT_BYTES */ }
impl LoadOptions { pub fn effective_max_bytes(&self) -> u64; }  // max_bytes clamped to the hard cap
impl Default for DocOrigin;  impl DocOrigin { pub fn display(&self) -> String; }   // canonical spec or "<memory>"
impl LineEnding { pub const fn as_text(self) -> &'static str; pub const fn as_str(self) -> &'static str; }
impl EolStyle { pub const fn as_str(self) -> &'static str; }
impl TextDocument {
    pub fn line_byte_range(&self, idx: LineIdx) -> Option<Range<usize>>;   // the line's content in the rope
    pub fn truncated(&self) -> bool;    // from_text cut text beyond D-43 (comparisons warn lossy_decode)
}
// rules
impl Default for TextRules / ImportanceRules / AlignmentRules;            // D-04
impl TextPattern { pub fn plain(text: impl Into<String>) -> TextPattern; pub fn regex(source: impl Into<String>) -> TextPattern; }
impl Display for UnimportantItem; impl TryFrom<String> for UnimportantItem; impl From<UnimportantItem> for String;
impl AlgorithmChoice {
    pub const ALL: [AlgorithmChoice; 6];
    pub const fn as_str(self) -> &'static str;                // the serde names; Display writes them
    pub fn parse(name: &str) -> Option<AlgorithmChoice>;      // ASCII-case-insensitive
    pub fn resolve(self, format: &CompiledFormat) -> Algorithm;  // before the D-36 size check (compare applies it)
}
impl TextRules { pub fn algorithm_for(&self, format: &CompiledFormat) -> Algorithm; }  // resolve() of the choice
// classify, compare, sections, rows
impl CharClass { pub const fn as_str(self) -> &'static str; }
impl RowState { pub const fn as_str(self) -> &'static str; }
impl Importance { pub const fn as_str(self) -> &'static str; }
impl SectionKind { pub const fn as_str(self) -> &'static str; }
pub fn compare_with_limits(input: CompareInput, limits: &RegexLimits, pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>, observer: Option<&mut dyn CompareObserver>) -> Result<TextComparison, TextError>;
pub const SPAN_LIMIT_BYTES: usize = 256 * 1024;   // bytes of a line that spans()/inline() classify
impl TextComparison {
    pub fn runs(&self) -> &[Run];                 // the alignment runs (stable run ids = indices, ARCH-050)
    pub fn bytes_identical(&self) -> bool;
    pub fn used_identity_path(&self) -> bool;     // D-53 (benches and tests)
}
// view, stats
impl Default for ViewOptions { /* All, iu off, no expanded ranges */ }
impl FilteredView {
    pub fn visible_runs(&self) -> impl Iterator<Item = Range<u32>> + '_;   // maximal runs of visible rows
    pub fn check_invariants(&self) -> Result<(), InvariantViolation>;      // INV-VIEW-1..3
}
impl TextStats { pub fn identities_hold(&self) -> bool; }                 // P-TC-5
// quick
#[derive(Clone, Copy, Default)]
pub struct QuickOptions<'a> {
    pub limits: RegexLimits,                                   // the registry's limits()
    pub external: Option<(&'a Path, &'a dyn ProcessRunner)>,   // as load_document takes it
}
pub fn quick_compare_bytes_with(left: (&[u8], &str), right: (&[u8], &str), format: &CompiledFormat,
    rules: &TextRules, load: &LoadOptions, opts: &QuickOptions<'_>, ctl: &Ctl<'_>)
    -> Result<ContentOutcome, Cancelled>;   // quick_compare_bytes = this with QuickOptions::default()
impl RulesComparator {
    pub fn rules(&self) -> &TextRules;
    pub fn with_external(self, temp_dir: PathBuf, runner: Arc<dyn ProcessRunner>) -> RulesComparator;
    pub fn with_stream_options(self, stream: StreamOptions) -> RulesComparator;  // byte comparisons (D-25, D-11)
}
// error
pub enum TextError { /* … */ Schema(SchemaError) /* a newer rules schema: classifies unsupported */ }
impl TextError {
    pub fn with_side(self, side: Side) -> TextError;   // sets the side of a Load error (load_document reports Left)
    pub fn is_cancelled(&self) -> bool;
}
impl From<Cancelled> for TextError;   // TextError::Cancelled
impl From<DiffError> for TextError;   // DiffError::Cancelled → TextError::Cancelled; others → TextError::Diff
```

**Accepted deviations (declared in wave 3):**

1. `load_document` does not know the side: VFS failures are `TextError::Load { side: Side::Left, .. }`; callers
   apply `TextError::with_side`.
2. `TextError::Schema` (a rules file with a newer `schema_version`, kind `unsupported`) and the hand-written
   `From<DiffError>` (cancellation is never an error state, T3) replace `Diff(#[from] DiffError)`.
3. An External conversion without a runner fails with `TextError::Convert` (kind `unsupported`) in `load_document`;
   the binary verdict of a converted document comes from the converted bytes. *Decision 7:* the quick compare and
   `RulesComparator` now apply External conversions exactly like `load_document` (`QuickOptions::external`,
   `RulesComparator::with_external`), and without a runner fail with the same error (§2.19 steps 0 and 1b).
4. Serde names follow `rename_all = "snake_case"` literally: `LineEnding::CrLf` and `EolStyle::CrLf` are `"cr_lf"`.
5. Key memory: the original grammar-path keys were ≈ 2 × text (G1 still passed). With the wave-3 encoding
   (§2.13.2) keys are ≈ 1 × text on both paths; the worst case is ≈ 2 × text + 2 B/line (3 × with more than 127
   elements) for token-dense lines whose separators are removed; §0.8 and §5.7 state it.
6. `RegexLimits`: `compare` and `quick_compare_bytes` keep using `RegexLimits::default()`; `compare_with_limits` and
   `quick_compare_bytes_with` take the session registry's limits, which `RulesComparator` uses (decision 8; TC-016
   holds when the session passes the same limits on every path).
7. `format_mismatch` is emitted by the caller that chooses the effective format (`Service::text_compare`,
   `RulesComparator`), not by `compare` (one effective format is its input); `TextComparison::warnings` says so.
8. `TextDocument::from_text` is total: text beyond the D-43 limits is cut; `TextDocument::truncated()` reports it
   and `compare` adds a `lossy_decode` warning naming the cut.
9. The thumbnail is built from the class runs (maximal runs of non-Same rows with one state and importance) in
   O(runs + buckets); with `ignore_unimportant` unimportant rows count as SAME (§2.18 `cat`); buckets that cover no
   row are empty. `next_section` is defined by section start rows (`Backward` from inside a section returns that
   section).
10. hikasa-text uses the `regex` crate (`regex::escape` of plain unimportant patterns, so they share §2.11's engine
    selection and case handling) and `encoding_rs` (the chunked decoder of decision 3); §0.5 lists both.
11. D-04 says that a default comparison is "exact apart from line endings, encoding and BOM": that holds for the
    elements a format marks important and case-sensitive; formats can declare elements case-insensitive (C
    numbers, SQL keywords) or ship them unimportant (Python `Whitespace`, COBOL `Identification`), and then the
    identity fast path does not apply (docs/05 §3 states it; D-04's text is unchanged).

**Amendments by decision:**

- *Decision 1 (row 15 and short lines):* the scorer of §2.15.3 (wave 3): 999 for texts equal after removing
  whitespace, char-multiset Dice when either text has fewer than 3 chars (`}` vs `};` = 666), bigram Dice
  otherwise, capped at 999. §2.13.3 row 15 is normative and holds with every engine (golden 18, key-table test).
- *Decision 2 (D-53, INV-KEY-3):* the key of §2.13.2 (wave 3) emits token boundaries and element ids only where an
  unimportant char was dropped; under all-important rules the general key equals the identity key byte for byte,
  for every grammar. Changed pairs of raw-equal lines are Changed/Unimportant (§2.17.1); the quick compare's step 5
  is guarded (§2.19).
- *Decision 3 (budgets and cancellation):* the closeness scorer works on 256-char samples under a deterministic unit
  budget (§2.15.2) and returns 0 after cancellation; `load_document` decodes in 1 MiB steps (UTF-8 validation,
  `encoding_rs` streaming decoder, UTF-32), sorts (Sorted) with a cancellable stable merge sort, counts lines and
  builds the line table and the rope in 1 MiB steps, runs a large encoding detection (≥ 8 MiB) on a helper thread
  polled every 10 ms, checks the token after the build and never returns `Ok` after a cancellation;
  `from_bytes` runs the same pipeline with `CancelToken::never()`. Unimportant-pattern matches are streamed
  (O(patterns) memory) with the token checked before every regex call and every 64 KiB of a line. Further
  uninterruptible O(n) steps found while measuring G4 were removed: each decoded piece goes through a small reused
  buffer (encoding_rs initialises one byte per page of the destination's spare capacity on every call, so a
  worst-case reservation of the whole output was faulted in at once); keys of ≥ 64 KiB are interned through
  surrogates of their exact equivalence classes (fingerprint and exact comparison in 1 MiB steps), so
  `intern_pair` never hashes a huge key in one call; lines of ≥ 64 KiB are compared piecewise from the ropes and
  copied into identity keys piecewise; and when a cancellation unwinds, buffers that can reach hundreds of MiB
  (raw bytes, decoded text, partial ropes and line tables, key arenas) are freed on a short-lived background
  thread. Residual bound: hikasa-formats' `tokenize_all` takes each line through `LineSource::line`, which copies
  a line that spans rope chunks in one step: O(line length), about 10–40 ms per 100 MiB line. Measured with
  `g4_cancel_latency_adversarial` (load ≈ 2): worst 37 ms. A session computing `bytes_identical` for `text compare`
  should compare the raw bytes in 1 MiB steps with cancellation checks and drop them off its thread, as the gate
  harness does (wave 4, §3.2).
- *Decision 4 (D-36):* `Auto` for source code is Histogram only within the fallback budget, else Standard.
- *Decision 5 (characters-per-line limit):* the key marks artificial breaks (`ART`), always; raw equality includes
  it (§2.17.1).
- *Decision 6 (inline):* every inline range of a Changed/Unimportant row is unimportant (§2.16 step 0).
- *Decision 8 (other findings):* `--unimportant "Everything Else"` (any ASCII case) is the word `everything-else`;
  `element:Everything Else` and a rules-file `elements` key `"Everything Else"` are `TextError::Rules`.
  `inline()`/`spans()` are bounded on long lines (§2.16, `SPAN_LIMIT_BYTES`). The D-43 line guard counts the pieces
  of wrapped lines before the table is allocated (and allocation failures are `TooLarge`). The quick compare's step 6
  does not repeat `lossy_decode`/`binary_content` (the per-side warnings name the sides). A right-side format that
  fails to compile no longer fails a `RulesComparator` pair when the left format is used (`format_mismatch` names
  the error). A future `DiffWarningCode` without a core counterpart is reported as `alignment_degraded` with its
  code in the message, never dropped. `RulesComparator::with_stream_options` lets the session pass its binary
  comparator's options (§3.3).

### 1.6 hikasa-folder

**Purpose.** Folder Compare: rules (FC-020…FC-027) with the script `criteria` vocabulary (D-57), filters
(FC-050…FC-054), name alignment (FC-010), symlink handling and flatten (FC-013, FC-014), and a parallel,
cancellable scanner (FC-016) that fills a **live `FolderModel`**. The model is readable during the scan
(ARCH-004, ARCH-082): per-folder counters make state, colour, visibility, statistics and verdict O(1) per folder
on partial trees too (FC-030, FC-055…FC-057, FC-064, D-55). Id-only events tell observers what changed. When the
scan ends, the model is frozen into a `FolderComparison`. Content tests go through the injected
`ContentComparator` objects under a shared `ByteBudget` (D-25); the crate does not depend on hikasa-text.
**Uses.** hikasa-core, hikasa-vfs, rayon, serde, thiserror, unicode-normalization, crossbeam-channel,
parking_lot, jiff, bitflags, toml.

Re-exports (§0.4 rule 8): `pub use hikasa_vfs::{Attrs, Capabilities, DosAttrs, EntryKind, FollowLinks, UnixMeta,
Vfs, VfsError};`

#### 1.6.1 Module tree

```
core/hikasa-folder/src/
├── lib.rs        re-exports
├── rules.rs      FolderRules, Criteria, CriteriaSpec, TimestampRule, TzOffset, UnixCriteria, ContentRule,
│                 NameAlignment, CaseMode, FilterRules, OtherFilters, SizeFilter, DateFilter, DateCutoff, Cmp,
│                 Handling, Change, FolderRulesPatch
├── resolve.rs    ResolvedRules (compiled NameFilter, absolute date cutoffs, case), resolve()
├── time_cmp.rs   TimeRelation, compare_times (§2.4)
├── align.rs      align_names, align_flat (§2.5)
├── quick.rs      DiffFlags, quick_tests (pure, per pair)
├── state.rs      CompareState, Category, state rules (§2.6.2, §2.6.3)
├── counts.rs     CategoryCounts, Aggregate, Aggregates, SideTotals (§2.6.4)
├── model/
│   ├── mod.rs    FolderModel, TreeRead, FolderComparison, CanonicalTree, BaseInfo
│   ├── node.rs   NodeId, NodeKind, NodeFlags, ScanStatus, ContentSummary, NodeView, EntryView, NodeError
│   └── store.rs  private storage: node arena with contiguous children, per-side entry tables, side tables
├── budget.rs     ByteBudget, BudgetGuard
├── scan.rs       compare_into, compare_folders, FolderEnv, ContentEngines (scanner on an explicit rayon pool)
├── events.rs     ScanEvent, ScanObserver, NullObserver, ChannelObserver
├── display.rs    FolderDisplayFilter, FolderToggles, FolderMode, DisplayOptions, VisibleRow, VisibleTree,
│                 visible_tree, ColourClass
├── stats.rs      FolderStats, ContentCounts
└── error.rs      FolderError
```

#### 1.6.2 Rules (persisted TOML, `hikasa folder compare --rules-file <file>`)

```rust
/// Schema version of folder rules files.
pub const FOLDER_RULES_SCHEMA_VERSION: u32 = 1;

/// Folder session rules. `Default` gives the values in comments. Every field round-trips through TOML
/// (INV-RULES-2): no `Option` field has a non-`None` default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderRules {
    /// Schema version.
    pub schema_version: u32,
    /// Comparison criteria.
    pub criteria: Criteria,
    /// Name alignment.
    pub alignment: NameAlignment,
    /// Filters.
    pub filters: FilterRules,
    /// Handling.
    pub handling: Handling,
}

/// Comparison criteria (FC-020…FC-027). The quick tests apply to file-like pairs only: folder pairs are compared
/// through their contents (§2.6.1 as amended, §1.6.8 item 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Criteria {
    /// FC-020. Default true.
    pub size: bool,
    /// FC-021. Default enabled, 2 s, no DST, no TZ, no offset.
    pub timestamps: TimestampRule,
    /// FC-022. Default false.
    pub filename_case: bool,
    /// FC-023 DOS attribute subset (empty = off). Default empty. Serialised as letters (`"HRS"`, D-30).
    pub dos_attributes: DosAttrs,
    /// FC-024. Default all false.
    pub unix: UnixCriteria,
    /// FC-025…FC-027.
    pub content: ContentRule,
}

impl Criteria {
    /// Parses the script `criteria` vocabulary (D-57, §2.6.6). Shared by `--criteria` and (M2) scripts.
    pub fn parse_list(list: &str) -> Result<CriteriaSpec, FolderError>;
}

/// Result of `Criteria::parse_list`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CriteriaSpec {
    /// Every criteria field is `Some`: listed criteria on, unlisted quick tests and the content method off;
    /// `follow_symlinks` is `Some(true)` only when listed, else `None`.
    pub patch: FolderRulesPatch,
    /// `ignore-unimportant` was listed (a display option, FC-057).
    pub ignore_unimportant: bool,
}

/// Timestamp test (FC-021, D-52). An explicit `enabled` flag instead of an `Option`, because TOML has no null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct TimestampRule {
    /// Compare timestamps at all. Default true.
    pub enabled: bool,
    /// Tolerance in seconds (inclusive). Default 2.
    pub tolerance_secs: u32,
    /// Exactly 1 h apart (± tolerance) counts as same. Default false.
    pub ignore_dst: bool,
    /// Any whole number of hours apart (|k| ≤ 26, ± tolerance) counts as same. Default false.
    pub ignore_timezone: bool,
    /// Shift one side's times by whole hours before comparing (script `timezone:left+2`). Default `None`,
    /// omitted from TOML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<TzOffset>,
}

/// Whole-hour shift applied to one side's timestamps (`side` is Left or Right; `hours` in −12..=12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TzOffset { pub side: Side, pub hours: i8 }

/// Unix metadata tests (FC-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UnixCriteria { pub permissions: bool, pub owner: bool, pub group: bool }

/// Content test (FC-025…FC-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentRule {
    /// Method. Default None.
    pub method: ContentMethod,
    /// FC-026. Default false (D-34).
    pub skip_if_quick_same: bool,
    /// FC-027 (D-48: cancels only TIME and SIZE differences). Default true.
    pub override_quick: bool,
}

/// Case handling for name alignment and masks (FC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    /// Sensitive iff both bases' back-ends report `CASE_SENSITIVE`.
    #[default]
    Auto,
    /// Always sensitive.
    Sensitive,
    /// Always insensitive.
    Insensitive,
}

/// Name alignment options (FC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NameAlignment {
    /// Case handling.
    pub case: CaseMode,
    /// Align `x.txt` with `x.csv` (files only).
    pub align_extensions: bool,
    /// Align NFC with NFD names.
    pub align_normalization: bool,
}

/// Comparison operator for other filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp { Less, Greater }

/// Size filter (`size:<10MB`); files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SizeFilter { pub cmp: Cmp, pub bytes: u64 }

/// Date cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DateCutoff {
    /// An absolute instant.
    Absolute { at: FileTime },
    /// N days before today's local midnight (FC-053: "relative to midnight"); resolved with `FolderEnv::now`.
    DaysAgo { days: u32 },
}

/// Date filter (`cutoff:<30days`); files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DateFilter { pub cmp: Cmp, pub cutoff: DateCutoff }

impl SizeFilter {
    /// Parses the script/CLI vocabulary `[<|>]<n>[KB|MB|GB|TB]` (D-37; `none` handled by the caller). Without an
    /// operator the filter keeps smaller files (`Less`, §1.6.8 item 11).
    pub fn parse(s: &str) -> Result<SizeFilter, FolderError>;
}
impl DateFilter {
    /// Parses `[<|>](<YYYY-MM-DD>|<RFC 3339>|<n>days)`. A bare date is local midnight in the zone of
    /// `zone` (the adapter's zone, injected; core never reads it). Without an operator the filter keeps files
    /// modified after the cutoff (`Greater`, "newer than", §1.6.8 item 11).
    pub fn parse(s: &str, zone: &jiff::tz::TimeZone) -> Result<DateFilter, FolderError>;
}

/// Other filters (FC-053 subset in M1: size, date, exclude protected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct OtherFilters {
    /// Exclude items with both SYSTEM and HIDDEN attributes. Default true (no effect without DOS attributes).
    pub exclude_protected: bool,
    /// Size filter. Default `None`.
    pub size: Option<SizeFilter>,
    /// Date filter. Default `None`.
    pub date: Option<DateFilter>,
}

/// Filters.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterRules {
    /// Name filters (FC-050).
    pub names: NameFilterLists,
    /// Other filters.
    pub other: OtherFilters,
}

/// Folder handling (FC-012…FC-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Handling {
    /// FC-013. Default false.
    pub follow_symlinks: bool,
    /// FC-014. Default false.
    pub flatten: bool,
    /// Scan orphan subfolders (headless: always useful for complete statistics). Default true.
    pub scan_orphan_folders: bool,
}

/// A patch value that can also clear an optional rule (serialised `"clear"` or `{"set": …}`), so that "clear"
/// survives JSON transport (IPC, D-50), unlike `Option<Option<T>>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change<T> { Clear, Set(T) }

/// CLI/IPC overrides (`None` = keep). Applied after `--rules-file` and `--criteria` (§3.5). Serialised with
/// `skip_serializing_if = "Option::is_none"` on every field.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderRulesPatch {
    pub size: Option<bool>,
    pub timestamps_enabled: Option<bool>,
    pub tolerance_secs: Option<u32>,
    pub ignore_dst: Option<bool>,
    pub ignore_timezone: Option<bool>,
    pub tz_offset: Option<Change<TzOffset>>,
    pub filename_case: Option<bool>,
    pub dos_attributes: Option<DosAttrs>,
    pub unix: Option<UnixCriteria>,
    pub content_method: Option<ContentMethod>,
    pub skip_if_quick_same: Option<bool>,
    pub override_quick: Option<bool>,
    pub case: Option<CaseMode>,
    pub align_extensions: Option<bool>,
    pub align_normalization: Option<bool>,
    /// Toolbar syntax (FC-050); replaces `filters.names` with the parsed lists.
    pub name_filter: Option<String>,
    pub exclude_protected: Option<bool>,
    pub size_filter: Option<Change<SizeFilter>>,
    pub date_filter: Option<Change<DateFilter>>,
    pub follow_symlinks: Option<bool>,
    pub flatten: Option<bool>,
    pub scan_orphan_folders: Option<bool>,
}

impl FolderRules {
    /// Parses a rules file (D-31).
    pub fn from_toml(text: &str) -> Result<(FolderRules, Vec<Warning>), FolderError>;
    /// Canonical TOML (round-trips: INV-RULES-2).
    pub fn to_toml(&self) -> String;
    /// Applies a patch (validates `name_filter` and ranges).
    pub fn apply(&mut self, patch: &FolderRulesPatch) -> Result<(), FolderError>;
}
```

#### 1.6.3 Model: nodes, views, counters

```rust
/// Node id: arena index, unique within one comparison; `NodeId::ROOT` is the base-folder pair. Ids may
/// differ between runs that use parallelism; structure and states never do (compare with `to_canonical`).
/// Ids are never printed (JSON and reports use paths).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The base-folder pair.
    pub const ROOT: NodeId = NodeId(0);
}

/// Node kind (kinds never mix within a node, D-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind { Folder, File, Symlink, Other }

/// Comparison state (FC-030). Canonical (computed with ignore_unimportant = false).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareState { Unknown, Same, LeftNewer, RightNewer, Different, LeftOrphan, RightOrphan, Error, Filtered }

/// Display category (state adjusted for ignore-unimportant; §2.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category { Same, LeftNewer, RightNewer, Different, LeftOrphan, RightOrphan, Unknown, Error, Filtered }

bitflags::bitflags! {
    /// Which quick or content tests found a difference (detail for columns, JSON, reports).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct DiffFlags: u16 {
        const SIZE = 1; const TIME = 2; const NAME_CASE = 4; const ATTRIBUTES = 8; const PERMISSIONS = 16;
        const OWNER = 32; const GROUP = 64; const CONTENT = 128; const LINK_TARGET = 256;
    }
}

bitflags::bitflags! {
    /// Per-node facts that are not states.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct NodeFlags: u16 {
        /// Hidden by a filter on the left (FC-054).
        const LEFT_FILTERED = 1;
        /// Hidden by a filter on the right.
        const RIGHT_FILTERED = 2;
        /// The left entry is name-only (D-45).
        const LEFT_STAT_SKIPPED = 4;
        /// The right entry is name-only.
        const RIGHT_STAT_SKIPPED = 8;
        /// A per-entry error or a failed listing on the left (details: `TreeRead::errors`).
        const LEFT_ERROR = 16;
        /// The same on the right.
        const RIGHT_ERROR = 32;
        /// A followed symlink closed a loop (D-33).
        const SYMLINK_LOOP = 64;
        /// The rules-based method fell back to bytes because of the size limit (D-25).
        const RULES_SKIPPED_SIZE = 128;
        /// A content test was requested but skipped because the quick tests said "same" (FC-026).
        const CONTENT_SKIPPED = 256;
    }
}

/// Timestamp relation (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeRelation { Same, LeftNewer, RightNewer }

/// Scan status of a node (`NotApplicable` for file-like nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus { NotApplicable, NotScanned, Scanned, Failed }

/// Compact content result of a node (the full outcome: `TreeRead::content_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentSummary {
    /// Method that ran (after fallbacks the method requested, e.g. `Rules` with `RULES_SKIPPED_SIZE`).
    pub method: ContentMethod,
    /// Result kind.
    pub kind: ContentKind,
}

/// A per-node error (failed listing, failed stat, failed content test, symlink loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    /// Side concerned (`None` = the pair).
    pub side: Option<Side>,
    /// The error.
    pub error: ErrorReport,
}

/// Borrowed view of one side's entry. Storage is private (§1.6.6); views keep the API stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryView<'a> {
    /// Name (UTF-8, lossy when `name_lossy`).
    pub name: &'a str,
    /// The OS name was not valid UTF-8.
    pub name_lossy: bool,
    /// Kind (of the target when `followed`).
    pub kind: EntryKind,
    /// Bytes (files); 0 for folders and name-only entries.
    pub size: u64,
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Attrs,
    /// Symlink target text.
    pub link_target: Option<&'a str>,
    /// The entry is a followed symlink.
    pub followed: bool,
    /// Back-end or computed CRC32.
    pub crc32: Option<u32>,
    /// Hidden by a filter on this side (FC-054).
    pub filtered: bool,
    /// Name-only entry (D-45): size 0, no times, `Attrs::None`.
    pub stat_skipped: bool,
    /// Flatten mode (FC-014 Path column): the folder containing the entry, relative to the base ("" = the base);
    /// `None` outside flatten mode.
    pub dir: Option<&'a str>,
}

/// Borrowed view of one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeView<'a> {
    /// Id.
    pub id: NodeId,
    /// Parent (`None` for the root).
    pub parent: Option<NodeId>,
    /// Kind.
    pub kind: NodeKind,
    /// Canonical state (folders: derived from their counters, §2.6.4).
    pub state: CompareState,
    /// Detail flags (all differences found, including those cancelled by override, D-48).
    pub diffs: DiffFlags,
    /// Timestamp relation, when compared.
    pub time: Option<TimeRelation>,
    /// Content result, when a content test ran.
    pub content: Option<ContentSummary>,
    /// Scan status.
    pub scan: ScanStatus,
    /// Flags.
    pub flags: NodeFlags,
    /// Left entry.
    pub left: Option<EntryView<'a>>,
    /// Right entry.
    pub right: Option<EntryView<'a>>,
}

/// Per-category counts (u32: one model holds at most `u32::MAX - 1` nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CategoryCounts {
    pub same: u32, pub left_newer: u32, pub right_newer: u32, pub different: u32,
    pub left_orphan: u32, pub right_orphan: u32, pub unknown: u32, pub error: u32, pub filtered: u32,
}

impl CategoryCounts {
    /// Count of one category.
    pub fn get(&self, c: Category) -> u32;
    /// left_newer + right_newer + different + left_orphan + right_orphan.
    pub fn differences(&self) -> u64;
    /// Sum of all counters.
    pub fn total(&self) -> u64;
}

/// Counts of a folder's descendants (D-55): file-like nodes in `files`, folder nodes (by their own category,
/// §2.6.4) in `folders`. The folder itself is not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Aggregate { pub files: CategoryCounts, pub folders: CategoryCounts }

/// Aggregates for both values of ignore-unimportant (FC-057 toggles without recomputation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Aggregates { pub all: Aggregate, pub ignoring_unimportant: Aggregate }

impl Aggregates {
    /// `all` or `ignoring_unimportant`.
    pub fn get(&self, ignore_unimportant: bool) -> &Aggregate;
}

/// Totals of present, non-filtered entries of one side in a subtree (FC-040 folder size, FC-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SideTotals { pub files: u64, pub folders: u64, pub bytes: u64 }

/// One base folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseInfo {
    /// Spec.
    pub spec: Spec,
    /// Entry of the base folder.
    pub entry: Entry,
    /// Back-end capabilities.
    pub capabilities: Capabilities,
}
```

#### 1.6.4 The live model, the read guard and the frozen comparison

```rust
/// The comparison model: created empty, filled by `compare_into`, readable at any time (D-55). `Send + Sync`;
/// shared as `Arc<FolderModel>` by the scanner, the GUI and reports.
pub struct FolderModel { /* specs, rules, parking_lot::RwLock<Store>, generation: AtomicU64, finished: AtomicBool */ }

impl FolderModel {
    /// Validates the rules (masks compile, ranges) and creates a model whose root is the base pair (state
    /// Unknown, `NotScanned`). No I/O.
    pub fn new(left: Spec, right: Spec, rules: FolderRules) -> Result<Arc<FolderModel>, FolderError>;
    /// Short read access. The scanner holds the write lock for one directory batch or one content result at a
    /// time, never during I/O or event delivery; readers must hold the guard for one GUI frame or one report
    /// chunk at most.
    pub fn read(&self) -> TreeRead<'_>;
    /// Incremented after every applied mutation (cheap change detection for the GUI).
    pub fn generation(&self) -> u64;
    /// True once `compare_into` returned (complete or cancelled).
    pub fn is_finished(&self) -> bool;
    /// The two base specs.
    pub fn specs(&self) -> &Pair<Spec>;
    /// The rules.
    pub fn rules(&self) -> &FolderRules;
}

/// A read guard with every query. It hides the lock type (no parking_lot type in the API).
pub struct TreeRead<'a> { /* parking_lot::RwLockReadGuard<'a, Store>, &'a FolderModel */ }

impl TreeRead<'_> {
    /// `NodeId::ROOT`.
    pub fn root(&self) -> NodeId;
    /// Number of nodes so far.
    pub fn len(&self) -> u32;
    /// A node (`None` for an id this model never issued).
    pub fn node(&self, id: NodeId) -> Option<NodeView<'_>>;
    /// Children in canonical order (§2.5.4); a contiguous id block, empty for files and unlisted folders.
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + '_;
    /// Relative path of a node (representative names, '/'-joined; the root is "").
    pub fn path(&self, id: NodeId) -> RelPath;
    /// Spec of a node's side, built from the base spec and the raw names (`None` when the side is absent or
    /// the base is not `Spec::Local` in M1). The GUI opens file pairs with it (FC-062).
    pub fn spec(&self, id: NodeId, side: Side) -> Option<Spec>;
    /// Display category (§2.6.3), O(1).
    pub fn category(&self, id: NodeId, ignore_unimportant: bool) -> Category;
    /// Descendant counters of a folder (zero for file-like nodes).
    pub fn aggregates(&self, id: NodeId) -> Aggregates;
    /// Per-side totals of a subtree (a file: its own size).
    pub fn totals(&self, id: NodeId) -> Pair<SideTotals>;
    /// Colour class (FC-031 mapping, §2.6.4), O(1).
    pub fn colour(&self, id: NodeId, side: Side, ignore_unimportant: bool) -> ColourClass;
    /// Errors of a node (empty when none).
    pub fn errors(&self, id: NodeId) -> &[NodeError];
    /// The full content outcome (method, result with error report, warnings), when a content test ran.
    pub fn content_outcome(&self, id: NodeId) -> Option<ContentOutcome>;
    /// Appends the visible rows in pre-order (§2.7); O(visible rows + children of expanded folders).
    /// `expanded` decides whether a visible folder's children are listed (CLI and reports: `|_| true`).
    pub fn visible_rows(&self, opts: &DisplayOptions, expanded: &dyn Fn(NodeId) -> bool, out: &mut Vec<VisibleRow>);
    /// Pre-order traversal (node, depth) over every node except the root; children in canonical order.
    pub fn preorder(&self) -> impl Iterator<Item = (NodeId, u32)> + '_;
    /// Statistics (§2.6.5), O(1) from the root counters.
    pub fn stats(&self, ignore_unimportant: bool) -> FolderStats;
    /// Overall verdict (§2.6.5, D-02), O(1).
    pub fn verdict(&self, ignore_unimportant: bool) -> CompareVerdict;
    /// Base folders once loaded (`Started` event).
    pub fn bases(&self) -> Pair<Option<&BaseInfo>>;
    /// Case sensitivity used for alignment and masks (after resolving `CaseMode::Auto`), once bases are loaded.
    pub fn case(&self) -> Option<CaseSensitivity>;
    /// True when the scan finished without cancellation.
    pub fn is_complete(&self) -> bool;
    /// Comparison-level warnings (capability missing for a criterion, symlink loops, entry errors …).
    pub fn warnings(&self) -> &[Warning];
}

/// The finished comparison: the live model after `compare_into` returned. No writes happen any more, so reads
/// never wait. `Send + Sync`; cheap to clone (`Arc` inside). INV-FOLDER-1..10 (Appendix B).
#[derive(Clone)]
pub struct FolderComparison { /* Arc<FolderModel> */ }
impl fmt::Debug for FolderComparison { /* specs, completeness and root statistics */ }

impl FolderComparison {
    /// The underlying model.
    pub fn model(&self) -> &Arc<FolderModel>;
    /// Read access (all queries).
    pub fn read(&self) -> TreeRead<'_>;
    /// Rules used.
    pub fn rules(&self) -> &FolderRules;
    /// False when cancelled (unscanned folders are `Unknown`).
    pub fn is_complete(&self) -> bool;
    /// Statistics.
    pub fn stats(&self, ignore_unimportant: bool) -> FolderStats;
    /// Overall verdict (D-02).
    pub fn verdict(&self, ignore_unimportant: bool) -> CompareVerdict;
    /// Id-free canonical form for equality checks (T2) and golden files.
    pub fn to_canonical(&self) -> CanonicalTree;
    /// Re-checks INV-FOLDER-1..10 (recounts every counter).
    pub fn check_invariants(&self) -> Result<(), InvariantViolation>;
}

/// Id-free canonical tree: nested `(name, kind, state, diffs, content kind, flags, children)`; implements
/// `PartialEq` and `Serialize` for golden files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalTree { /* … */ }
```

#### 1.6.5 Scanning, budget, events

```rust
/// Content engines given to the scanner (hikasa-text's RulesComparator is passed as `rules`).
#[derive(Clone)]
pub struct ContentEngines {
    /// CRC method.
    pub crc: Arc<dyn ContentComparator>,
    /// Binary method.
    pub binary: Arc<dyn ContentComparator>,
    /// Rules method (None → `FolderError::RulesComparatorMissing` when requested).
    pub rules: Option<Arc<dyn ContentComparator>>,
}

/// Execution environment of a folder comparison.
pub struct FolderEnv<'a> {
    /// VFS (normally `VfsRouter`).
    pub vfs: &'a dyn Vfs,
    /// Content engines.
    pub content: &'a ContentEngines,
    /// Pool for listing and content tests (None = current thread).
    pub pool: Option<&'a rayon::ThreadPool>,
    /// Cancellation + progress.
    pub ctl: Ctl<'a>,
    /// Incremental events.
    pub observer: &'a dyn ScanObserver,
    /// "Now" in the user's zone (date filters relative to midnight; D-24).
    pub now: &'a jiff::Zoned,
    /// Content-test memory budget in bytes (D-25). The session passes `content_memory_budget_mib` × 2^20.
    pub content_budget: u64,
}

/// Scans `model` (created by `FolderModel::new`, never scanned before: else `AlreadyScanned`). Loads the bases
/// (errors → `BaseNotLoadable` / `BaseNotFolder`), then fills the model (§2.5–§2.6, §3.3). Cancellation after the
/// bases are loaded returns `Ok` with `is_complete() == false`. The bases are loaded when both are stat'ed and
/// listed. The model is marked finished on every exit path, unwinding included (then also incomplete; §1.6.8).
pub fn compare_into(model: &Arc<FolderModel>, env: &FolderEnv<'_>) -> Result<FolderComparison, FolderError>;

/// `FolderModel::new` + `compare_into` (headless callers).
pub fn compare_folders(left: &Spec, right: &Spec, rules: &FolderRules, env: &FolderEnv<'_>)
    -> Result<FolderComparison, FolderError>;

/// A counting semaphore over bytes (parking_lot `Mutex` + `Condvar`), shared by all content tests of one scan.
#[derive(Debug)]
pub struct ByteBudget { /* capacity, Mutex<u64>, Condvar */ }

impl ByteBudget {
    /// A budget of `capacity` bytes (≥ 1).
    pub fn new(capacity: u64) -> ByteBudget;
    /// Takes `min(bytes, capacity)` bytes, blocking while they are not available. Waits in 10 ms slices
    /// (`Condvar::wait_for`) to observe `cancel` (returns `Err(Cancelled)`). The guard returns the bytes on drop.
    pub fn acquire(&self, bytes: u64, cancel: &CancelToken) -> Result<BudgetGuard<'_>, Cancelled>;
}

/// Returns its bytes to the budget on drop.
#[derive(Debug)]
pub struct BudgetGuard<'a> { /* &'a ByteBudget, bytes */ }

/// Incremental scan events (ARCH-004). They carry ids only (hints, §1.10); consumers read details from the model
/// and re-read the ancestors of the nodes they refresh (`TreeRead::ancestors`), whose counters change with every
/// batch. Ordering guarantees INV-EVT-1..3 (Appendix B, as amended in §1.6.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// The base folders were loaded (`TreeRead::bases` is set); the root exists.
    Started,
    /// A folder pair was listed and aligned; its children now exist as one contiguous block, with their quick
    /// test states.
    FolderListed { folder: NodeId },
    /// Nodes whose state, content result or flags changed since they were last announced (deduplicated batch
    /// of at most 256 ids; the counters of their ancestors changed too and are not named: consumers re-read them).
    NodesUpdated { nodes: Vec<NodeId> },
    /// The scan ended (`complete == false` after cancellation). Always the last event.
    Finished { complete: bool },
}

/// Receives events from worker threads, after the mutation they report is visible in the model. Must not block
/// for long (use `ChannelObserver`).
pub trait ScanObserver: Send + Sync {
    /// One event.
    fn on_event(&self, event: ScanEvent);
    /// False for observers that ignore events; the scanner then builds no event at all (no allocation on the
    /// CLI path). Default true.
    fn wants_events(&self) -> bool { true }
}

/// Discards events (CLI); `wants_events()` is false.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullObserver;
impl ScanObserver for NullObserver { /* no-op; wants_events = false */ }

/// Forwards events to a bounded crossbeam channel. The scanner then applies backpressure, so a slow GUI slows
/// the scan instead of growing memory (ARCH-082).
pub struct ChannelObserver { tx: crossbeam_channel::Sender<ScanEvent> }

impl ChannelObserver {
    /// Creates the observer and its receiver (`capacity` ≥ 1).
    pub fn bounded(capacity: usize) -> (ChannelObserver, crossbeam_channel::Receiver<ScanEvent>);
}
impl ScanObserver for ChannelObserver { /* send; a disconnected receiver drops events silently */ }
```

#### 1.6.6 Storage (normative bounds; layout private)

The layout behind the views is private, so it can become a struct-of-arrays later without API changes. M1
MUST meet these bounds (bench `folder_model_1m`, §6.4):

- children of a folder are allocated as one contiguous id block when the folder is listed;
- a node plus both sides' metadata take at most **256 bytes**, excluding the name bytes (names live in one
  string arena per side; link targets, CRCs, errors, content outcomes with warnings and flatten folders live in
  side tables keyed by `NodeId` and cost nothing when absent);
- every folder stores its `Aggregates` (2 × 18 × 4 bytes) and its `Pair<SideTotals>` (48 bytes).

#### 1.6.7 Display and statistics

```rust
bitflags::bitflags! {
    /// Atomic folder display categories (FC-055 toggle mode).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FolderToggles: u8 {
        const SAME = 1; const LEFT_ORPHANS = 2; const RIGHT_ORPHANS = 4;
        const LEFT_NEWER = 8; const RIGHT_NEWER = 16; const DIFFERENCES = 32;
    }
}

/// Display filter presets (FC-055) plus toggle mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderDisplayFilter {
    All, Differences, Same, NoOrphans, DifferencesNoOrphans, Orphans, LeftNewer, RightNewer,
    LeftNewerAndLeftOrphans, RightNewerAndRightOrphans, LeftOrphans, RightOrphans, None,
    /// Toggle mode with explicit `FolderToggles` bits.
    Toggles { bits: u8 },
}

impl FolderDisplayFilter {
    /// Equivalent toggle set (table in §2.7).
    pub fn toggles(self) -> FolderToggles;
    /// Parses CLI values (`differences-no-orphans`) and script names (`display-mismatches-no-orphans`).
    pub fn parse(s: &str) -> Result<FolderDisplayFilter, FolderError>;
}

/// Folder display mode (FC-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderMode {
    /// Always Show Folders.
    AlwaysShowFolders,
    /// Compare Files and Folder Structure.
    #[default]
    CompareStructure,
    /// Only Compare Files.
    OnlyFiles,
}

/// Display options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayOptions {
    /// Filter.
    pub filter: FolderDisplayFilter,
    /// Folder mode.
    pub folders: FolderMode,
    /// FC-057 Ignore Unimportant Differences.
    pub ignore_unimportant: bool,
    /// FC-057 Suppress Filters: reveals items hidden by file filters, in teal (D-44).
    pub suppress_filters: bool,
}

/// A visible row of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisibleRow {
    /// Node.
    pub node: NodeId,
    /// Depth (root children = 0).
    pub depth: u32,
}

/// Visible rows in pre-order (random access for a virtualised tree).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibleTree { rows: Vec<VisibleRow> }

impl VisibleTree {
    /// Rows.
    pub fn rows(&self) -> &[VisibleRow];
    /// Number of rows.
    pub fn len(&self) -> usize;
}

/// `TreeRead::visible_rows` collected (CLI and reports pass `|_| true`).
pub fn visible_tree(cmp: &FolderComparison, opts: &DisplayOptions, expanded: &dyn Fn(NodeId) -> bool) -> VisibleTree;

/// Colour class of a node on one side (FC-031 mapping, §2.6.4; actual colours are theme data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColourClass { Same, NewerOrDifferent, Orphan, OlderOrUnknown, Filtered, Error, Absent }

/// Content result counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContentCounts {
    pub binary_same: u32, pub rules_same: u32, pub unimportant_only: u32, pub important_differences: u32,
    pub binary_different: u32, pub error: u32, pub not_compared: u32,
}

/// Folder statistics (FC-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FolderStats {
    /// File-like pairs by category.
    pub files: CategoryCounts,
    /// Folder pairs by category (the root excluded).
    pub folders: CategoryCounts,
    /// Left totals.
    pub left: SideTotals,
    /// Right totals.
    pub right: SideTotals,
    /// Content test results.
    pub content: ContentCounts,
}

/// Folder errors (only failures of the whole comparison; per-node failures are node states).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FolderError {
    /// A base folder cannot be stat'ed or listed (exit 107).
    #[error("{side} base folder {spec}: {source}")]
    BaseNotLoadable { side: Side, spec: String, #[source] source: VfsError },
    /// A base spec is not a folder (exit 107).
    #[error("{side} base {spec} is not a folder")]
    BaseNotFolder { side: Side, spec: String },
    /// Invalid filters.
    #[error("filters: {0}")]
    Filter(#[from] MaskError),
    /// Invalid rules, filter expression or criteria word.
    #[error("rules: {0}")]
    InvalidRules(String),
    /// `ContentMethod::Rules` without a rules comparator (configuration bug).
    #[error("rules-based content comparison requested but no comparator was provided")]
    RulesComparatorMissing,
    /// `compare_into` was called on a model that was already scanned.
    #[error("this folder model was already scanned")]
    AlreadyScanned,
    /// Cancelled before the base folders were loaded.
    #[error("operation cancelled")]
    Cancelled,
}
```

#### 1.6.8 Accepted additions and deviations (wave 3)

The reviews of wave 3 (conformance and adversarial) and the maintainer's decisions accepted the following public
items, deviations and amendments. They are part of the contract: later waves (report, session, GUI) may use them
and must not assume the text above where an item here refines it. The algorithm sections (§2.3–§2.7), §6.2.6 and
Appendix B carry the same amendments in place.

**Additive API** (no signature above changes; no breaking change):

```rust
pub enum FolderError {
    // … the variants of §1.6.7, plus:
    /// A rules file written by a newer version (`schema_version` newer than `FOLDER_RULES_SCHEMA_VERSION`, D-31,
    /// SES-052). Classified `unsupported`, as `SchemaError` itself (§1.1.12).
    #[error("folder rules: {0}")]
    Schema(#[from] SchemaError),
}
impl FolderRules {
    /// The checks of `FolderModel::new` (masks compile, ranges).
    pub fn validate(&self) -> Result<(), FolderError>;
}
impl Cmp {
    /// `Less`: `a < b`; `Greater`: `a > b`.
    pub fn holds<T: PartialOrd>(self, a: T, b: T) -> bool;
}
impl<T> Change<T> {
    /// `Set(v)` → `Some(v)`, `Clear` → `None`.
    pub fn into_option(self) -> Option<T>;
}

/// The alignment and quick-test functions §1.6.1 names, with their inputs.
pub struct AlignInput<'a> { pub name: &'a FileName, pub kind: EntryKind, pub dir: &'a str }
pub struct AlignOptions { pub case: CaseSensitivity, pub align_extensions: bool, pub align_normalization: bool }
pub fn align_names(left: &[AlignInput<'_>], right: &[AlignInput<'_>], opts: &AlignOptions)
    -> Vec<Pair<Option<usize>>>;                                         // canonical child order (§2.5.4)
pub fn align_flat(left: &[AlignInput<'_>], right: &[AlignInput<'_>], opts: &AlignOptions)
    -> Vec<Pair<Option<usize>>>;
pub fn quick_tests(left: &Entry, right: &Entry, criteria: &Criteria) -> (DiffFlags, Option<TimeRelation>);

impl DiffFlags { pub fn names(self) -> Vec<&'static str>; }           // JSON form of bit sets (§4.1 rule 9)
impl NodeFlags {
    pub fn names(self) -> Vec<&'static str>;
    pub const fn filtered(side: Side) -> NodeFlags;
    pub const fn stat_skipped(side: Side) -> NodeFlags;
    pub const fn error(side: Side) -> NodeFlags;
}
impl CompareState {
    pub const ALL: [CompareState; 9];
    pub const fn as_str(self) -> &'static str;
    pub const fn category(self) -> Category;
    pub const fn mirrored(self) -> CompareState;                         // orphans and newer swap
}
impl Category {
    pub const ALL: [Category; 9];
    pub const fn index(self) -> usize;
    pub const fn as_str(self) -> &'static str;
    pub const fn is_difference(self) -> bool;                            // counted by `differences()`
    pub const fn state(self) -> CompareState;
}
impl NodeKind {
    pub const fn of(kind: EntryKind) -> NodeKind;
    pub const fn entry_kind(self) -> EntryKind;
    pub const fn as_str(self) -> &'static str;
}
impl ScanStatus { pub const fn as_str(self) -> &'static str; }
impl TimeRelation { pub fn mirrored(self) -> TimeRelation; }
/// The pure §2.6.2/§2.6.3 functions.
pub mod state {
    pub const OVERRIDABLE: DiffFlags;                                    // SIZE | TIME (D-48)
    pub fn base_state(diffs: DiffFlags, time: Option<TimeRelation>) -> CompareState;
    pub fn pair_state(diffs: DiffFlags, time: Option<TimeRelation>, content: Option<ContentKind>,
                      override_quick: bool) -> (CompareState, DiffFlags);
    pub fn state_if_rules_same(diffs: DiffFlags, time: Option<TimeRelation>, override_quick: bool)
        -> CompareState;
    pub fn file_category(state: CompareState, diffs: DiffFlags, time: Option<TimeRelation>,
                         content: Option<ContentKind>, override_quick: bool, ignore_unimportant: bool)
        -> Category;
}
impl CategoryCounts { pub fn plus(&self, other: &CategoryCounts) -> CategoryCounts; }
impl Aggregate { /** `files + folders`, field-wise */ pub fn both(&self) -> CategoryCounts; }
impl ContentCounts { pub fn get(&self, kind: ContentKind) -> u32; }
impl ByteBudget { pub fn capacity(&self) -> u64; pub fn available(&self) -> u64; }
impl BudgetGuard<'_> { pub fn bytes(&self) -> u64; }
impl ContentEngines {
    /// The hikasa-vfs CRC and binary comparators with `stream` (the caller chooses `threads`), plus `rules`.
    pub fn with_stream_options(stream: StreamOptions, rules: Option<Arc<dyn ContentComparator>>) -> ContentEngines;
    pub fn get(&self, method: ContentMethod) -> Option<&Arc<dyn ContentComparator>>;
}
impl FolderToggles {
    pub const fn atom(category: Category) -> Option<FolderToggles>;    // None for Filtered (§2.7)
    pub fn accepts(self, category: Category) -> bool;
}
impl FolderDisplayFilter { pub const PRESETS: [FolderDisplayFilter; 13]; }
impl FolderMode { pub const ALL: [FolderMode; 3]; }
impl VisibleTree { pub fn is_empty(&self) -> bool; }
impl<'a> NodeView<'a> {
    pub fn side(&self, side: Side) -> Option<&EntryView<'a>>;
    pub fn representative(&self) -> Option<&EntryView<'a>>;             // left when present
    pub fn is_folder(&self) -> bool;
}
impl TreeRead<'_> {
    pub fn is_empty(&self) -> bool;
    pub fn rules(&self) -> &FolderRules;
    /// Ids of `id`'s ancestors, nearest first (the event consumer protocol of INV-EVT-3).
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId>;
}
/// INV-FOLDER-1..10 on a live model (P-FOLDER-6).
pub fn check_tree_invariants(tree: &TreeRead<'_>) -> Result<(), InvariantViolation>;
/// The layout of `CanonicalTree` (golden files).
pub struct CanonicalTree { pub complete: bool, pub root: CanonicalNode }
pub struct CanonicalNode {
    pub name: String, pub kind: NodeKind, pub state: CompareState, pub scan: ScanStatus, pub diffs: Vec<String>,
    pub time: Option<TimeRelation>, pub content: Option<ContentKind>, pub flags: Vec<String>,
    pub errors: Vec<CanonicalError>, pub left: Option<CanonicalEntry>, pub right: Option<CanonicalEntry>,
    pub children: Vec<CanonicalNode>,
}
pub struct CanonicalEntry { pub name: String, pub kind: EntryKind, pub size: u64, pub dir: Option<String> }
pub struct CanonicalError { pub side: Option<Side>, pub kind: ErrorKind }
/// Depth limit of a scan that follows symbolic links (D-33).
pub const MAX_FOLLOW_DEPTH: u32 = 256;
// Debug: FolderModel, TreeRead, FolderEnv, ContentEngines, ChannelObserver. Clone: ChannelObserver.
// `#[doc(hidden)] FolderModel::reserved_bytes` serves the `folder_model_1m` bench and is not API.
```

**Deviations, clarifications and amendments** (the maintainer's wave-3 decisions are marked *decision n*):

1. **`FolderError::Schema`** (above): a rules file from a newer version is `Schema`, not `InvalidRules`.
2. **Folder category order** (§2.6.4 amended, *decision 3*): `own_category(F)` is, first match wins: no present
   side → Unknown; every present side filtered → Filtered (also for an orphan, §2.3); `ScanStatus::Failed` →
   Error, **also when the folder has one side only** (a failure is never claimed as an orphan, D-02); a child of a
   failed listing → Unknown; one side only → LeftOrphan / RightOrphan; `NotScanned` → Unknown; then the counters.
   A folder's colour follows the same order (`Failed` → Error before Orphan).
3. **Precedence of file-like nodes** (§2.6.2 amended, *decision 4*): every present side filtered → Filtered, also
   when an error is attached (the side flag, the `NodeError` and the `entry_error`/`symlink_loop` warning stay, so
   nothing is silent); then a per-entry error → Error; a child of a failed listing → Unknown; one side → orphan;
   else the quick-test/content state. An entry known by name only because its stat failed (`Listing::errors`
   without a matching entry) is filtered when the name masks exclude its name (its kind is unknown: the file
   masks); it is never name-only (INV-FOLDER-7 amended). An excluded broken link therefore never makes the verdict
   Incomplete.
4. **Pairing of entries whose stat failed** (*decision 5*): they pair by name with each other or with an orphan of
   the other side — same folder and exact name (flatten), exact name, then the case-folded exact name when
   case-insensitive — through hash maps (O(n) per level, cancellable), candidates taken in (exact name, folder)
   order, so the result does not depend on the listing order.
5. **No quick tests on folders** (*decision 7*; §2.6.1 amended; docs/04 §3): folders are compared by aggregating
   their contents. A folder's own size, time, name case, DOS attributes, permissions, owner and group are not
   compared; `diffs` and `time` of a folder node are always empty/`None`.
6. **Bases loaded = stat'ed and listed**: a failure of a base's listing is `BaseNotLoadable` (a cancellation
   meanwhile `Cancelled`), before any event.
7. **Capability switch-off for both sides**: a criterion whose capability either base lacks is switched off for the
   whole comparison (also on the side that has it), with one `capability_missing` warning per (criterion, side
   lacking it); a stat'ed entry without a modification time on a back-end that reports `MTIME` adds one
   `capability_missing` warning per side.
8. **Depth cap only when following links**: `MAX_FOLLOW_DEPTH` (256, D-33) applies with `follow_symlinks` only;
   without links the file system bounds the depth and the scanner is iterative.
9. **Flatten specifics** (FC-014; *decision 3*): (a) every folder of each side is collected first; the files are
   then aligned, planned and inserted as **one batch** under the root, with the folder table in the same mutation —
   a flattened scan shows no live rows, and a cancelled one inserts nothing; (b) folders do not appear, except a
   folder that could not be listed, closed a loop or hit the depth limit: a one-sided Folder row with
   `ScanStatus::Failed` and category Error (two failed folders of one path pair); (c) the other side's files at or
   below a failed folder's path (paths matched with the alignment's case and normalisation options) are not
   aligned: they are Unknown rows, never orphans (§2.6.2), so the error is counted and the verdict is Incomplete
   unless real differences exist — as in the folder tree; (d) a folder hidden by `exclude_protected` only (not by
   the name masks) is collected once a folder of the same path was collected on the other side (rounds until
   nothing changes), so a folder protected on one side only does not turn its files into orphans, while a folder
   protected on every side stays hidden.
10. **Exclude protected and folders** (§2.3 amended, *decision 2*): a folder side is listed iff its **name-mask**
    verdict is not `Excluded`; in addition, a folder whose present sides are all filtered (by any filter: SYSTEM +
    HIDDEN on every side, a protected orphan) is not listed (state Filtered). A folder protected on one side only is
    listed on both sides, its protected side flagged filtered (FC-054), and identical contents give NoDifferences
    (exit 2), not orphans.
11. **Operator-less filters**: `SizeFilter::parse` without `<`/`>` keeps smaller files (`Less`); `DateFilter::parse`
    without an operator keeps newer files (`Greater`).
12. **TOML forms**: `DosAttrs` as letters (D-30); `DateCutoff` tagged `type = "absolute" | "days_ago"`;
    `timestamps.offset`, `filters.other.size` and `filters.other.date` are omitted when `None` (TOML has no null);
    `Change` is `"clear"` / `{"set": …}` in JSON. The serde form of `FolderRules` is this TOML form, so the `rules`
    object embedded in `hikasa.folder_compare.v1` (§4.3.3) would omit those keys, whereas §4.1 rule 3 wants
    absent optionals as `null`: that is the report DTO's concern (hikasa-report renders `rules` through its own
    DTO, T5) and is flagged for wave 4; `FolderRules`' serde form does not change.
13. **P-FOLDER-5 amendment**: "more exclusions never add visible nodes" holds for the display filter All, and for
    Differences when no FC-054 pair can arise. With extension alignment, a newly filtered side turns a
    content-tested pair into an FC-054 pair, which gets no content test (§2.6.2), so D-48 can no longer cancel its
    TIME/SIZE differences and the pair can become visible under Differences.
14. **Criteria vocabulary** (§2.6.6): only the documented spellings are accepted — the tolerance unit is `sec`
    (`timestamp:3sec`; no `s`, `secs` or bare number), the offset sides are `left` and `right` (no `lt`/`rt`).
15. **Names that are not valid Unicode** (§2.5.1 amended): `k2` folds the exact OS form (valid UTF-8 runs folded,
    other bytes kept), never the lossy text; such a name has no `k3`/`k4` beyond `k2`; `NAME_CASE` compares the same
    folded exact forms. Two different non-UTF-8 names never pair and never "differ only in case".
16. **Link cycles reported as `ELOOP`** (D-33): an entry error that is the back-end's report of a symbolic-link
    cycle (the OS error `ELOOP`, or `ERROR_CANT_RESOLVE_FILENAME` on Windows, in its message or causes, as
    LocalVfs passes it on and MemoryVfs imitates it, §1.2.7 item 5) gives the node `SYMLINK_LOOP` and the warning
    `symlink_loop`, like a followed link back to an ancestor, in both scan modes.
17. **Quick tests on filtered pairs** (§2.3 as written): a pair whose present sides are all filtered and which has
    two stat'ed sides (size, date or attribute filters) gets its quick tests (`diffs`, `time`); its state stays
    Filtered and no content test runs.
18. **Cancellation inside a batch** (§5.3, *decision 6*): every step of a folder batch — building the items, each
    alignment level, the special pairing, the planning (and before every FC-054 re-stat), the canonical order and
    the preparation of the nodes — polls the token at least every 1,024 entries. A batch cancelled before its
    insertion is dropped: the folder stays `NotScanned`/Unknown and the scan reports `complete == false`; an
    insertion is all or nothing (INV-FOLDER-3). The nodes of a batch are prepared outside the write lock (with
    touched node chunks for large batches), and the lock only appends them. With a pool, a large batch is freed on
    the pool (`ThreadPool::spawn`, fire and forget) so that returning after a cancellation does not wait for a
    million entries to be freed; without a pool it is freed on the calling thread (tens of milliseconds for a
    million entries, the one step a pool-less scan cannot interrupt). Measured with the `cancel_latency` bench
    (release, 8-thread pool, shared 16-thread machine, load average 7–20): the 100k scan of G2 cancelled
    half-way ≤ 5 ms; one folder of 1,000,000 entries per side cancelled at 15 points of its batch ≤ 20 ms
    (load ≈ 8) and ≤ 51 ms (load ≈ 17); 200,000 re-stats cancelled half-way ≤ 0.6 ms (was 1.1 s and 0.23 s,
    with `complete == true`).
19. **Memory per batch** (§5.7): a batch keeps each listed entry once (the listing's own vector, with a parallel
    vector of scan facts of ~24 B) and ~64 B per planned child (rare parts boxed); keys are sorted as indices; the
    listing is freed before the node chunks are prepared. One folder of 1,000,000 entries per side peaks at
    0.72 GiB above the start without a pool and 0.78 GiB with one (the pool frees the listing concurrently), the
    two listings (2 × 200 MB) and the model (185 MB) included — was 1.97 GiB. `folder_model_1m` is unchanged
    (161.3 B per node excluding names, gate ≤ 256).
20. **Finishing**: `compare_into` marks the model finished on every exit path, unwinding included (and
    incomplete unless the scan returned normally), so a GUI polling the shared model never waits for a scan that
    ended with a panic.
21. **INV-EVT-3 amended** (*decision 8*; the consumer protocol was chosen over naming every ancestor): events are
    id-only hints (§1.10). A consumer that mirrors the tree re-reads, at each event, the named nodes, the children of
    listed folders, and the ancestors of both (`TreeRead::ancestors`): ancestors' counters — and so their states —
    change with every batch and content result and are not named. Replaying under this protocol ends with exactly
    `to_canonical()` (P-FOLDER-4).
22. **Tests read no ambient zone** (T1, *decision 1*): tests inside `src/` build zones from POSIX rules
    (`TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3")`), never from the system tz database, and assert
    unconditionally; architecture check 2 holds for the crate.
23. **`unicode-normalization`** is declared in the manifest (the §0.5 row) but not used by hikasa-folder: NFC comes
    from `hikasa_core::case::nfc`. The §0.5 row should say "declared but unused in M1" (or the dependency be
    dropped); §0.5 is outside this crate's sections and is left to the maintainer.
24. **Per-field unix criteria** (added by the wave-4 session review, decision 7, §1.8.7 item 19): `FolderRulesPatch`
    has `unix_permissions`, `unix_owner` and `unix_group: Option<bool>` (skip-serialised when `None`), applied
    after `unix`, so `--permissions`, `--owner` and `--group` change one test each and the rules file's or the
    criteria list's other unix tests stay (§3.5: individual flags are the highest layer, per flag). `unix`
    (`Criteria::parse_list` sets it: unlisted → off) is unchanged. Additive; tested in `rules.rs`
    (`unix_patch_fields_are_independent`) and in hikasa-session.

### 1.7 hikasa-report

**Purpose.** Pure rendering from result models to bytes (RPT-002, RPT-004, RPT-010, RPT-011). The output is
byte-identical for identical inputs on every platform: `\n` line ends (CSV uses `\r\n`, RFC 4180), no locale,
no clock (time is injected through `ReportMeta`), and column widths from `unicode-width`. The crate also owns
the stable JSON DTOs of the comparison results, because the JSON folder report and `--json` are the same
document (§4).
**Uses.** hikasa-core, hikasa-diff, hikasa-text, hikasa-folder, serde, serde_json, jiff, thiserror,
unicode-width, bitflags. Format and VFS types are named through the hikasa-text and hikasa-folder re-exports
(§0.4 rule 8).

#### 1.7.1 Module tree

```
core/hikasa-report/src/
├── lib.rs
├── meta.rs         ReportMeta (title, injected time, generator, mode, filters, per-side errors), header (RPT-004)
├── options.rs      DisplayMode, OutputFormat, Wrap, mapping helpers (SCR-001 vocabulary)
├── escape.rs       HTML escaping, CSV quoting, control-character escaping for text output
├── text/
│   ├── mod.rs      TextLayout, PatchStyle, TextReportOptions, render_text_report
│   ├── unified.rs  unified diff (annotated default and plain patch)
│   ├── context.rs  context diff
│   ├── normal.rs   normal diff
│   ├── sbs.rs      side-by-side plain text
│   ├── html.rs     side-by-side HTML (self-contained, legend, "N lines hidden" rows)
│   ├── summary.rs  summary (text/HTML)
│   └── stats.rs    statistics CSV
├── folder/
│   ├── mod.rs      FolderLayout, FolderColumns, FolderReportOptions, render_folder_report
│   ├── list.rs     status list (CLI default: one record per line)
│   ├── sbs.rs      side-by-side plain text
│   ├── html.rs     side-by-side HTML
│   └── summary.rs  summary (text/HTML)
├── json/
│   ├── mod.rs      schema ids, shared DTO helpers (timestamps, line numbers)
│   ├── text.rs     TextCompareJson + builder
│   └── folder.rs   FolderCompareJson + builder
└── error.rs        ReportError
```

#### 1.7.2 Public API

```rust
/// Report header data (RPT-004: title, produced timestamp, mode, filters in effect, specs, per-side errors).
/// `generated_at = None` omits the line (golden tests). The specs come from the result model itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMeta {
    /// Title (default "Text Compare" / "Folder Compare").
    pub title: Option<String>,
    /// Generation time and zone, injected by the caller (D-24).
    pub generated_at: Option<jiff::Zoned>,
    /// "hikasa 0.1.0".
    pub generator: String,
    /// One-line mode summary ("display: differences; ignore unimportant: no"); `None` omits the line.
    pub mode: Option<String>,
    /// One-line summary of the rules/filters in effect ("size, timestamps (2 s), content: rules;
    /// filters: *.c;-obj\"); `None` omits the line.
    pub filters: Option<String>,
    /// Per-side load errors (a side that failed to load in a GUI view); M1 CLI reports have none.
    pub errors: Pair<Option<String>>,
}

impl ReportMeta {
    /// Header for a text report: `mode` from `opts`, `filters` from the rules' importance and alignment
    /// summary (deterministic wording, §3.4).
    pub fn for_text(cmp: &TextComparison, opts: &TextReportOptions, generated_at: Option<jiff::Zoned>,
                    generator: &str) -> ReportMeta;
    /// Header for a folder report: `mode` from `opts.display`, `filters` from criteria, name and other filters.
    pub fn for_folder(cmp: &FolderComparison, opts: &FolderReportOptions, generated_at: Option<jiff::Zoned>,
                      generator: &str) -> ReportMeta;
}

/// Report display modes (RPT-003, SCR-001 `display-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DisplayMode {
    /// display-all.
    All,
    /// display-mismatches.
    Mismatches,
    /// display-context (text: ± lines; folders: differences with their folder structure).
    Context { lines: u32 },
    /// display-matches.
    Matches,
}

impl DisplayMode {
    /// Text filter: All→All, Mismatches→Differences, Context→Context, Matches→Same.
    pub fn text_filter(self) -> TextDisplayFilter;
    /// Folder filter: All→(All, CompareStructure), Mismatches→(Differences, OnlyFiles),
    /// Context→(Differences, CompareStructure), Matches→(Same, OnlyFiles).
    pub fn folder_filter(self) -> (FolderDisplayFilter, FolderMode);
}

/// Output encoding of a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// Plain text; `ansi` adds SGR colours (red important, blue unimportant, …).
    Text { ansi: bool },
    /// Self-contained HTML; `mono` uses the monochrome stylesheet (RPT-002 html-mono).
    Html { mono: bool },
}

/// Line wrapping in side-by-side text (RPT-002 wrap none/character/word). HTML always wraps by CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Wrap { None, #[default] Character, Word }

/// Patch styles (TP-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchStyle {
    /// Normal (`a`/`d`/`c` commands).
    Normal,
    /// Context diff with N context lines.
    Context { lines: u32 },
    /// Unified diff with N context lines.
    Unified { lines: u32 },
}

/// Text report layouts (13-reports, M1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextLayout {
    /// CLI default: unified diff whose hunk headers carry ` important` / ` unimportant` (still a valid patch).
    AnnotatedUnified { lines: u32 },
    /// Side-by-side.
    SideBySide,
    /// Summary counts.
    Summary,
    /// Plain patch.
    Patch(PatchStyle),
    /// Statistics CSV (one header row + one data row).
    Statistics,
}

/// Text report options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextReportOptions {
    /// Layout.
    pub layout: TextLayout,
    /// Display mode (side-by-side and summary; patches always use their own context).
    pub display: DisplayMode,
    /// Treat unimportant as same (TC-024).
    pub ignore_unimportant: bool,
    /// Include line numbers (side-by-side). Default true.
    pub line_numbers: bool,
    /// Total width of side-by-side text (columns). Default 160.
    pub width: u16,
    /// Wrapping (side-by-side text).
    pub wrap: Wrap,
    /// Output format (HTML allowed for SideBySide and Summary only).
    pub format: OutputFormat,
}

/// Renders a text report into `out`.
pub fn render_text_report(
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    out: &mut dyn Write,
) -> Result<(), ReportError>;

bitflags::bitflags! {
    /// Folder report columns (13-reports; defaults SIZE | MODIFIED).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FolderColumns: u16 {
        const STATUS = 1; const SIZE = 2; const MODIFIED = 4; const ATTRIBUTES = 8;
        const CRC = 16; const OWNER = 32; const GROUP = 64;
    }
}

/// Folder report layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FolderLayout {
    /// CLI default: `<marker> <path>` lines plus a `#` summary line (§4.3.3).
    List,
    /// Side-by-side table.
    SideBySide,
    /// Counts per state (13-reports).
    Summary,
    /// The `hikasa.folder_compare.v1` JSON document.
    Json,
}

/// Folder report options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderReportOptions {
    /// Layout.
    pub layout: FolderLayout,
    /// Display options (filter, folder mode, ignore-unimportant, suppress filters).
    pub display: DisplayOptions,
    /// Columns (SideBySide).
    pub columns: FolderColumns,
    /// Output format (HTML allowed for SideBySide and Summary).
    pub format: OutputFormat,
}

/// Renders a folder report into `out`.
pub fn render_folder_report(
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    out: &mut dyn Write,
) -> Result<(), ReportError>;

/// Builds the text JSON DTO (§4.3.2).
pub fn text_compare_json(cmp: &TextComparison, view: &ViewOptions) -> json::TextCompareJson;
/// Builds the folder JSON DTO (§4.3.3).
pub fn folder_compare_json(cmp: &FolderComparison, display: &DisplayOptions) -> json::FolderCompareJson;

/// Report errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReportError {
    /// The layout cannot produce the requested format (e.g. HTML patch).
    #[error("layout {layout} does not support {format}")]
    Unsupported { layout: &'static str, format: &'static str },
    /// Writing failed (`BrokenPipe` included: callers decide the policy, D-35).
    #[error("write failed: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialisation failed (bug).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
```

#### 1.7.3 Accepted additions and deviations (wave 4)

The wave-4 review of hikasa-report (adversarial and conformance findings, maintainer decisions 1–8) accepted the
following public items, interpretations, deviations and amendments. They are part of the contract: hikasa-session
(wave 4), hikasa-cli (wave 5) and the GUI build on them and must not assume the text above where an item here
refines it. No signature of §1.7.2 changes; every change is additive. Amended normative text: §3.4 (report
layouts), §4.3.2 (`display`), §4.3.3 (the JSON folder report, name escaping, the summary line), §6.2.7 and
Appendix B (INV-RPT-2). Where §2.18 says that the patch renderers use exactly the Context view, item 1 below
refines it for patches.

**Additive API** (complete list beyond §1.7.2):

```rust
// Modules `folder`, `json` and `text` are public; the items below are also re-exported at the crate root,
// except the DTO building blocks, which stay in `json`.
pub enum ReportError {
    // … the variants of §1.7.2, plus:
    /// A word of the report vocabulary is not recognised (SCR-001; T11: the parsers live here).
    #[error("invalid {option} {value:?} (expected {expected})")]
    InvalidOption { option: &'static str, value: String, expected: &'static str },
}
impl Classify for ReportError;   // Unsupported → unsupported; InvalidOption → invalid_input; Io → io_error_kind;
                                 // Json → internal
// The vocabulary parsers (ASCII case-insensitive, `_` equals `-`, the SCR-001 prefixes accepted) and their names.
impl DisplayMode { pub fn parse(word: &str, context_lines: u32) -> Result<DisplayMode, ReportError>;
                   pub fn describe(self) -> String; }               // "all", "differences", "context (3 lines)", "same"
impl Wrap { pub fn parse(word: &str) -> Result<Wrap, ReportError>; pub const fn as_str(self) -> &'static str; }
impl OutputFormat { pub fn parse(word: &str) -> Result<OutputFormat, ReportError>;   // text, ansi, html, html-mono
                    pub const fn name(self) -> &'static str; }
impl PatchStyle { pub fn parse(word: &str, lines: u32) -> Result<PatchStyle, ReportError>;
                  pub const fn as_str(self) -> &'static str; }
impl TextLayout { pub fn parse(word: &str, patch: PatchStyle, lines: u32) -> Result<TextLayout, ReportError>;
                  pub const fn name(self) -> &'static str; }       // the `layout` of ReportError::Unsupported
impl FolderLayout { pub fn parse(word: &str) -> Result<FolderLayout, ReportError>;
                    pub const fn name(self) -> &'static str; }
impl FolderColumns { pub fn parse_list(list: &str) -> Result<FolderColumns, ReportError>;
                     pub fn names(self) -> Vec<&'static str>; }
pub fn parse_folder_mode(word: &str) -> Result<FolderMode, ReportError>;   // --folders always|structure|files-only
pub const fn folder_mode_word(mode: FolderMode) -> &'static str;
// Defaults (13-reports): DisplayMode::All; OutputFormat::Text { ansi: false }; FolderColumns SIZE | MODIFIED;
// TextReportOptions = AnnotatedUnified { lines: 3 }, display All, ignore-unimportant off, line numbers, width 160,
// character wrapping, plain text; FolderReportOptions = List, display (All, CompareStructure, iu off, suppress
// off), SIZE | MODIFIED, plain text.
impl Default for DisplayMode; impl Default for OutputFormat; impl Default for FolderColumns;
impl Default for TextReportOptions; impl Default for FolderReportOptions;
impl ReportMeta { #[must_use] pub fn with_title(self, title: impl Into<String>) -> ReportMeta; }   // --title
/// Styling inputs that do not fit the `Copy` options: RPT-002 `html-custom`. API only in M1 (no CLI flag).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct HtmlStyle { pub custom_css: Option<String> }
impl HtmlStyle { pub fn custom(css: impl Into<String>) -> HtmlStyle; }
/// `render_text_report` with a style; returns the rendering's own warnings (`patch_lossy_eol`,
/// `patch_encoding`), which the report header also shows.
pub fn render_text_report_with(cmp: &TextComparison, opts: &TextReportOptions, meta: &ReportMeta,
                               style: &HtmlStyle, out: &mut dyn Write) -> Result<Vec<Warning>, ReportError>;
pub fn render_folder_report_with(cmp: &FolderComparison, opts: &FolderReportOptions, meta: &ReportMeta,
                                 style: &HtmlStyle, out: &mut dyn Write) -> Result<(), ReportError>;
pub mod json {
    pub const TEXT_COMPARE_SCHEMA: &str = "hikasa.text_compare.v1";
    pub const FOLDER_COMPARE_SCHEMA: &str = "hikasa.folder_compare.v1";
    pub struct FormatJson { pub id: Option<String>, pub name: String, pub kind: &'static str,
                            pub reason: Option<DetectReasonJson> }
    impl FormatJson { pub fn new(format: &CompiledFormat, reason: Option<&DetectReason>) -> FormatJson; }
    pub struct DetectReasonJson { pub reason: &'static str, pub id: Option<String>, pub mask: Option<String> }
    impl TextCompareJson {
        /// Sets `format.reason` and `detection`, which only the caller that chose the format knows.
        pub fn with_detection(self, format: Option<&DetectReason>, left: Option<&DetectReason>,
                              right: Option<&DetectReason>) -> TextCompareJson;
    }
    // Every DTO of §4.3.2 and §4.3.3 is a public struct with public fields, built by `new` constructors
    // (TextDisplayJson, TextSideJson, TextRulesJson, FolderRulesJson, FolderDisplayJson, EntryJson, …).
    pub struct FolderDisplayJson { /* …, */ pub toggles: Vec<&'static str> }   // additive field (item 6)
    /// The JSON folder report: `schema` first, then the fields of FolderCompareJson (decision 4).
    #[derive(Serialize)]
    pub struct FolderReportJson<'a> { pub schema: &'static str, #[serde(flatten)] pub result: &'a FolderCompareJson }
    impl<'a> FolderReportJson<'a> { pub fn new(result: &'a FolderCompareJson) -> FolderReportJson<'a>; }
}
```

`WarningCode::PatchEncoding` (`"patch_encoding"`, §1.1.11) is added in hikasa-core for item 2.

**Amendments (maintainer decisions 1–6).**

1. *Exact patches (decision 1).* A row is written as patch context only when its two lines are byte-for-byte
   identical: a `Same` row whose two terminators are equal and whose written BOM (item 2) is the same. Every other
   row is written as its deletion and its insertion, whatever the rules say (a `Same` row whose endings differ
   under D-04, or whose last line lacks the final newline on one side only, is a change of the patch). Hunks are
   the maximal runs of such rows ("groups"), clustered when at most `2 × N` identical rows apart and widened by
   `N` identical rows on each side (clamped to the file); with line endings compared and no BOM difference this
   is exactly the Context view of §2.18. Inside a hunk each group is written as all its left lines, then all its
   right lines. Context lines (both parts of a context diff included) are written from the left file. The normal
   diff writes one command per group with `N = 0` (one per differing row for a `lines_independent` format, like
   its sections, §2.17.2). Consequences: `\ No newline at end of file` only follows the true last line of a side
   and never sits in the middle of a hunk (a `-` line so marked can only be followed by `+` lines); every style
   (unified, annotated unified, context, normal) reproduces the right file byte for byte when applied to the left
   file, under any rules; the patch of a comparison that is "same" under the rules but not byte-identical has
   hunks. Ignore Unimportant drops the clusters without an important row (the patch then deliberately does not
   carry them); the rows of the kept clusters are exact. A hunk's annotation is ` important` when it holds an
   important row, else ` unimportant` (also for a hunk of ending-only differences); ANSI colours them red and
   blue. In a context diff an empty range after line `l` is written `l+1,l` (`0` at the start): patch programs
   read a lone non-zero number as one line, so the `diff -c` form `l` makes them reject a zero-context deletion.
2. *Encodings (decision 2).* For a UTF-8 file with a BOM, the patch writes U+FEFF at the start of line 1
   wherever line 1 appears (context, deletion or insertion), so the patch applies to the bytes. Other sources
   stay UTF-8 text, and the rendering warns `patch_encoding` once per affected side: a side stored in another
   encoding than UTF-8 (windows-1252, UTF-16, …) or decoded lossily (`malformed > 0`), whenever the files
   differ (the patch has a hunk, or the bytes differ: two files whose text is the same in different encodings
   give no hunk); and a side holding nothing but a UTF-8 BOM when the other side does not (no line can carry
   it). Lone-CR endings and artificial breaks are still written as LF with `patch_lossy_eol` (a context line
   counts for both sides); the patch then reproduces the right file with those endings as LF.
3. *P-RPT-1 oracle (decision 3).* `diffy` is used only for context ≥ 1: for a zero-context hunk with an empty
   new range it starts its search one line early and can delete the wrong one of two duplicate lines from a
   correct patch. Zero-context patches, and every style at every context, are checked with position-exact,
   byte-exact appliers written in the tests; the property is extended to the default rules (line endings not
   compared, or compared) with random LF, CRLF and lone-CR endings, missing final newlines and BOMs; GNU patch
   (no fuzz, every style) and `git apply` (unified) check a fixed corpus when on PATH (a missing tool is
   reported on stderr; `HIKASA_REQUIRE_PATCH_TOOLS=1` makes it a failure). The failing seeds are committed in
   `tests/proptest-regressions/props.txt`, and the review's two shrunk cases are explicit regression tests.
4. *JSON (decision 4).* `display.toggles` of the text document is always an array: the toggle bits in toggles
   mode, else the categories the mode shows (`all`: `same`, `left_orphans`, `right_orphans`, `differences`;
   `mismatches` and `context`: the three difference categories, `context` adding the same rows around them;
   `matches`: `same`; `none`: `[]`). The field keeps its type `Option<Vec<&str>>`; `new` always sets it and
   `None` is written as `[]`. `display.lines` stays `null` except for `context`. The JSON folder *report*
   (`FolderLayout::Json`) writes `"schema": "hikasa.folder_compare.v1"` as its first key (§4.1 rule 10); under
   `--json` the envelope carries the schema and `result` is the bare `FolderCompareJson`.
5. *Escaping (decision 5).* Bidirectional formatting characters (U+061C, U+200E, U+200F, U+202A–U+202E,
   U+2066–U+2069) are escaped (`\u{202e}`) wherever control characters are: names, headers, side-by-side text and
   HTML, ANSI patches. Names (and flatten-mode folders) in the folder list and side-by-side reports also escape a
   backslash as `\\`, so every printed name maps back to exactly one name; symbolic-link targets and header values
   escape control and bidirectional characters only. With ANSI colours, patches escape the control (except TAB)
   and bidirectional characters of the file content, as the side-by-side report does; plain patches keep the
   bytes.
6. *Side-by-side (decision 6).* Widths are `unicode-width` string widths of character clusters: a base character
   with its combining marks, variation selectors, emoji modifiers, tag characters and ZWJ-joined successors, and
   a pair of regional indicators, is one unit of the string's width that never straddles a break (no new
   dependency). Side-by-side text and HTML number lines by source line (`TextDocument::physical_line`): the pieces
   of a line broken by a characters-per-line limit show that line's number. The folder side-by-side shows blank
   columns for an entry of which only the name is known (name-only, D-45, folders included; or a failed stat:
   the side's error flag with no time, no attributes and size 0, which is all the scanner keeps), and a blank
   Size for a folder with an error on that side (its listing failed: the total is unknown).

**Accepted interpretations and deviations.**

- *Patch header.* Patches start with the RPT-004 header, every line prefixed `# ` (patch programs skip leading
  text; no title or path can read as patch syntax); the generation line is `Generated: <RFC 3339 with offset> by
  <generator>`, omitted when `generated_at` is `None`. `Mode:` shows the patch's own context (`display: context (N
  lines)`; the normal diff `display: differences`).
- *Ignore Unimportant in patches:* hunks and normal-diff commands without an important row are dropped (item 1).
- *CR and artificial breaks* are written as LF, with `patch_lossy_eol` (item 2).
- *JSON `display`:* `lines` is `null` outside `context`; `toggles` per item 4. The folder DTO's additive
  `display.toggles` lists the categories of the filter (never null). JSON `line` numbers and section line
  ranges stay logical (1-based indices of the document's lines), so that they agree with each other and with the
  rows; only the human side-by-side views show source lines under a characters-per-line limit (item 6).
- *`~` marker and summary line.* Text side-by-side `~` marks an unimportant change (` ` under Ignore
  Unimportant); the folder list `~` marks a "different" item whose differences are all unimportant (`=` under
  Ignore Unimportant, where it counts as same). The list's `#` summary line counts files (the `files` counters of
  the statistics) and is left out when the comparison holds no item. It is report output on stdout: `--quiet`
  keeps it (CLI invariant 3 of §1.9.2 wins over the former "`--quiet` omits it" of §4.3.3, amended).
- *`HtmlStyle`* carries the custom CSS (RPT-002 `html-custom`), embedded after the built-in stylesheet with `</`
  neutralised and control characters CSS-escaped; API only in M1 (no CLI flag).
- *Defaults* as listed in the API block.
- *Goldens* compare inputs with the Everything Else format only (no hikasa-formats dev-dependency, §0.5);
  grammar-dependent paths (another tab size, `lines_independent` normal diffs, the `format: <name>` wording,
  `format_mismatch`) are covered by hikasa-session Service tests and the wave-5 CLI goldens. The goldens hold
  Unix host paths and are `#[cfg_attr(not(unix), ignore)]` elsewhere (visible as ignored). ANSI goldens:
  `expected.annotated.ansi`, `expected.context.ansi`, `expected.sbs.ansi.txt` (three text cases) and
  `expected.list.ansi.txt`, `expected.sbs.ansi.txt` (two folder cases).
- *TOML-subset reader.* The report tests read the hikasa-folder `tree.toml` fixtures with a small reader of the
  TOML subset they use (the crate has no `toml` dependency).
- *Read guard (§3.4, §5.2).* The folder renderers collect the visible rows by one traversal under one read
  guard, then format them reading the model in chunks of 4096 rows, each under its own guard. Accepted for M1:
  the traversal is linear and a finished comparison has no writer waiting.
- *No `Ctl` (§5.3, §5.4 vs §1.7.2).* The renderers take no cancel token and no progress sink, as §1.7.2
  specifies, so a report is not cancellable and reports no `reporting` progress, contrary to the general text of
  §5.3 and §5.4. Accepted for M1: rendering is one linear pass over a finished model, and `--output` writes
  atomically, so no partial report appears. A `Ctl`-taking variant (with a `Cancelled` error) is additive later.
- *Module tree.* Beyond §1.7.1: the private modules `cells.rs` (display cells, cluster widths, the side-by-side
  wrapper), `style.rs` (SGR sequences, the stylesheets, the HTML skeleton) and `text/patch.rs` (the hunk planner
  of item 1, ranges, line writing and the patch warnings, shared by the three patch writers).
- *Not in M1:* the XML dump, interleaved layouts, printing and links to file reports (13-reports).

**Session-review additions (wave 4).** The review of hikasa-session (§1.8.7) added two things here:

- *Public escapers* (decision 7): `escape_controls`, `escape_terminal` and `escape_name` (item 5's rules) are
  public at the crate root, unchanged, so adapters escape terminal and list output exactly as the reports do
  (hikasa-session's `api::human` uses them). Additive.
- *Goldens* (the "grammar-dependent paths" bullet above): the session's own warnings — `format_mismatch` and the
  unknown keys of rules files — reach the JSON envelope and stderr through `TextCompareResponse::warnings`; they are
  not written into the RPT-004 header, which keeps the comparison's and the rendering's warnings. The
  grammar-dependent paths (another tab size, a `lines_independent` normal diff, the `format: <name>` wording, the
  title in text and folder reports) are covered by hikasa-session Service tests.

### 1.8 hikasa-session

**Purpose.** The service API used identically by the CLI and the GUI (ARCH-013). Also: configuration location
(SES-050), format-registry loading (SES-051, FF-004), M1 program options, atomic config writes (SES-053), job
control and job spawning, the shared legacy `gui open` argument parser (D-51), and the IPC protocol with its
client and server (ARCH-090…092). It is the only core crate that may log (`tracing`). Its `host` module is the
only place that reads the process environment, and only adapters call it.
**Uses.** hikasa-core, -vfs, -diff, -formats, -text, -folder, -report, serde, serde_json, toml, directories,
interprocess, thiserror, tracing, rayon, jiff, parking_lot, crossbeam-channel.

#### 1.8.1 Module tree

```
core/hikasa-session/src/
├── lib.rs
├── host.rs          EnvFacts, Platform, Clock, FixedClock, SystemClock, capture_env()  ← adapter-only
├── paths.rs         ConfigLocation, ConfigMode, PORTABLE_MARKER, resolve_config_dir, default_config_dir (pure)
├── options.rs       ProgramOptions (M1 subset of options.toml), OptionsLoad, load_options, OPTION_SECTIONS
├── formats.rs       FormatStore (load: built-ins + formats/*.toml + formats/order.toml; materialize)
├── atomic.rs        write_atomic (temp + fsync + rename)
├── jobs.rs          JobControl, JobHandle, run_guarded (catch_unwind boundary)
├── api/
│   ├── mod.rs       Service, ServiceInit, ApiOp
│   ├── requests.rs  QuickRequest, QuickMethod, TextCompareRequest, FolderCompareRequest, FormatTokensRequest
│   ├── responses.rs QuickResponse, TextCompareResponse, FormatChoice, PreparedFolderCompare,
│   │                FolderCompareResponse
│   ├── dto.rs       JsonEnvelope + DTOs for quick, format, vfs, gui and version (§4)
│   ├── gui.rs       GuiOpenRequest, GuiOpenOutcome, GuiLauncher, GuiLaunch, LaunchError, OpenArgs,
│   │                normalize_legacy_args, parse_open_args, infer_view_kind, gui_exe_candidates, find_gui_exe,
│   │                ViewWords, ResolvedOpen, ViewRequest, ViewOutcome, open_usage (§1.8.7)
│   ├── vocab.rs     parse_folder_mode, parse_content_method, parse_algorithm, parse_line_range,
│   │                resolve_text_display, resolve_folder_display (§1.8.7)
│   ├── human.rs     human lines of the list commands (§1.8.7)
│   └── error.rs     ApiError
└── ipc/
    ├── mod.rs       PROTOCOL_VERSION, MAX_FRAME_LEN
    ├── protocol.rs  Envelope, Request, Response, ViewKind, ViewId, OpenOptions, ReadOnly, ViewInfo, ViewResult,
    │                CloseReason, ServerInfo, IpcErrorCode
    ├── framing.rs   write_frame, read_frame, send, recv_request, recv_response (two-phase decode, D-46)
    ├── endpoint.rs  Endpoint, EndpointAddress, InstanceLock, fnv1a64 (§2.23)
    ├── server.rs    IpcServer, IpcHandler, ViewWaiter, ServerHandle
    ├── client.rs    IpcClient, RetryPolicy, connect_or_launch
    └── error.rs     IpcError
```

#### 1.8.2 Host facts, configuration, options

```rust
/// Platform family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform { Linux, MacOs, Windows, OtherUnix }

/// Every ambient fact Hikasa uses, captured ONCE by the adapter (T1). Tests construct it literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFacts {
    /// Platform.
    pub platform: Platform,
    /// Current directory.
    pub cwd: Option<PathBuf>,
    /// Home directory.
    pub home: Option<PathBuf>,
    /// Folder of the running executable (portable marker, GUI lookup).
    pub exe_dir: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `$XDG_CACHE_HOME`.
    pub xdg_cache_home: Option<PathBuf>,
    /// `$XDG_RUNTIME_DIR`.
    pub xdg_runtime_dir: Option<PathBuf>,
    /// `%APPDATA%`.
    pub appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub local_appdata: Option<PathBuf>,
    /// Temp directory (`std::env::temp_dir()`; on macOS the per-user `$TMPDIR`).
    pub temp_dir: PathBuf,
    /// User name (`$USER`/`%USERNAME%`), for the Windows pipe name.
    pub user: Option<String>,
    /// Effective uid (Unix). std only: Linux reads `/proc/self/status`; other Unix systems read the owner of a
    /// file `capture_env` creates and removes in `temp_dir`. `None` on Windows or on failure.
    pub uid: Option<u32>,
    /// Codeset from `LC_ALL`/`LC_CTYPE`/`LANG` ("UTF-8", "KOI8-R" …); `None` → UTF-8.
    pub locale_encoding: Option<String>,
    /// `$HIKASA_IPC_ENDPOINT` (tests, unusual setups).
    pub ipc_endpoint_override: Option<OsString>,
    /// `$PATH` (GUI executable lookup).
    pub path_var: Option<OsString>,
    /// `$HIKASA_GUI_EXE`: the GUI executable to launch instead of the candidates (tests; §1.8.7).
    pub gui_exe_override: Option<PathBuf>,
    /// `$NO_COLOR` is set to a non-empty value (§1.8.7).
    pub no_color: bool,
}

/// Reads the real process environment (`std::env`, `std::env::current_exe`, `directories::BaseDirs` for the home
/// folder). ADAPTER-ONLY: the architecture test fails if anything under `core/` calls it (except hikasa-session's
/// own tests).
pub fn capture_env() -> EnvFacts;

/// Source of "now" (D-24).
pub trait Clock: Send + Sync {
    /// Current time in the user's zone.
    fn now(&self) -> jiff::Zoned;
}
/// System clock and zone (adapter-only, like `capture_env`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;
/// Fixed time (tests, reproducible reports).
#[derive(Debug, Clone)]
pub struct FixedClock(pub jiff::Zoned);

/// Name of the portable-mode marker file (SES-050).
pub const PORTABLE_MARKER: &str = "hikasa-portable.toml";

/// How the configuration folder was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMode { Explicit, Portable, User }

/// The configuration folder and the cache folder (§2.22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    /// Folder (may not exist yet; read paths tolerate absence).
    pub dir: PathBuf,
    /// Mode.
    pub mode: ConfigMode,
    /// Cache folder (spooled stdin for `gui open -`, D-49); created on first write only.
    pub cache_dir: PathBuf,
    /// True when `dir` equals the platform default (no endpoint suffix, D-47).
    pub is_default: bool,
}

/// Resolves the configuration folder (§2.22). Pure: `exists` is injected.
pub fn resolve_config_dir(
    facts: &EnvFacts,
    explicit: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Result<ConfigLocation, SessionError>;
/// The platform default configuration folder (§2.22 step 3), or `None` when an ingredient is missing.
pub fn default_config_dir(facts: &EnvFacts) -> Option<PathBuf>;

/// Top-level sections of `options.toml` (docs/15 pages). Unknown keys inside them are ignored silently;
/// unknown top-level sections produce warning `unknown_config_key` (D-31).
pub const OPTION_SECTIONS: &[&str] = &[
    "startup", "tabs", "appearance", "text_editing", "next_difference", "backups", "file_operations",
    "archive_types", "commands", "open_with", "tweaks",
];

/// M1 subset of `options.toml` (15-options-reference). All other keys are preserved by not touching the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramOptions {
    /// `[text_editing] number_of_context_lines` (default 3).
    pub context_lines: u32,
    /// `[tweaks.file_operations] buffer_size_for_binary_compare` (default 1 MiB; clamped per `StreamOptions`).
    pub binary_buffer_size: u32,
    /// `[tweaks.file_operations] content_memory_budget_mib` (default 1024; D-25; added to docs/15 in wave 4).
    pub content_memory_budget_mib: u32,
}

/// Result of `load_options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsLoad {
    /// Options (defaults where missing).
    pub options: ProgramOptions,
    /// `unknown_config_key` warnings for unknown top-level sections (D-31).
    pub warnings: Vec<Warning>,
    /// Dotted names of every ignored key, for `--verbose` logging (never a warning).
    pub ignored_keys: Vec<String>,
}

/// Loads `options.toml` (missing file → defaults; newer `schema_version` → error).
pub fn load_options(config: &ConfigLocation) -> Result<OptionsLoad, SessionError>;

/// User formats folder (`<config>/formats`).
#[derive(Debug, Clone)]
pub struct FormatStore { dir: PathBuf }

impl FormatStore {
    /// Store for a configuration.
    pub fn new(config: &ConfigLocation) -> FormatStore;
    /// Built-ins merged with user files (§2.12.4). Broken user files are skipped with `user_format_skipped`.
    pub fn load(&self, limits: RegexLimits) -> Result<(FormatRegistry, Vec<Warning>), SessionError>;
    /// Writes the built-ins and `order.toml` when absent (GUI first run; D-21). Never overwrites.
    pub fn materialize(&self) -> Result<(), SessionError>;
}

/// Writes `bytes` to `path` atomically (temp in the same folder, fsync, rename) (SES-053).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()>;

/// Session-level errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// No configuration folder can be determined.
    #[error("cannot determine the configuration folder (no home directory)")]
    NoConfigDir,
    /// `--config-dir` is relative and the current directory is unknown.
    #[error("--config-dir {0:?} must be absolute")]
    RelativeConfigDir(PathBuf),
    /// Config file problem.
    #[error("{path}: {message}")]
    Config { path: String, message: String },
    /// Format problem.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// I/O.
    #[error("{path}: {source}")]
    Io { path: String, #[source] source: std::io::Error },
    /// A configuration file of a newer version (§1.8.7 item 4).
    #[error("{path}: {source}")]
    Schema { path: String, #[source] source: SchemaError },
    /// `--config-dir ""` (§1.8.7 item 30).
    #[error("--config-dir must not be empty")]
    EmptyConfigDir,
}
```

#### 1.8.3 Jobs

```rust
/// Cancellation + progress of one service call. The GUI keeps the `CancelToken` to stop the job.
#[derive(Clone)]
pub struct JobControl {
    /// Id.
    pub id: JobId,
    /// Kind.
    pub kind: JobKind,
    /// Cancellation.
    pub cancel: CancelToken,
    /// Progress sink (wrapped by the service in a `ThrottledSink`, ≤ 20 Hz per phase).
    pub progress: Arc<dyn ProgressSink>,
}

impl JobControl {
    /// Borrowed control for engine calls.
    pub fn ctl(&self) -> Ctl<'_>;
}

/// A job started with `Service::spawn` (the GUI's job API; the CLI calls service methods directly).
pub struct JobHandle<T> { /* id, kind, title, cancel, progress: Receiver<Progress>, result: Receiver<Result<T, ApiError>> */ }

impl<T> JobHandle<T> {
    /// Id.
    pub fn id(&self) -> JobId;
    /// Kind.
    pub fn kind(&self) -> JobKind;
    /// Title given to `spawn`.
    pub fn title(&self) -> &str;
    /// Requests cancellation.
    pub fn cancel(&self);
    /// Lossy progress stream: a bounded channel (capacity 64) whose sender drops reports when it is full, so a
    /// slow GUI never blocks a job.
    pub fn progress(&self) -> &crossbeam_channel::Receiver<Progress>;
    /// The result, if the job has finished (non-blocking; `Some` at most once).
    pub fn try_result(&self) -> Option<Result<T, ApiError>>;
    /// True once the job has finished (additive, §1.8.7).
    pub fn is_finished(&self) -> bool;
    /// Blocks until the job finishes; after `try_result` returned the result: `InvalidArgument` (§1.8.7).
    pub fn wait(self) -> Result<T, ApiError>;
}

/// Runs `f` inside `std::panic::catch_unwind`; a panic becomes `ApiError::Internal("panic: …")` (§5.6).
pub fn run_guarded<T>(f: impl FnOnce() -> Result<T, ApiError>) -> Result<T, ApiError>;
```

#### 1.8.4 Service API (`hikasa_session::api`)

```rust
/// Service construction parameters (captured by the adapter).
pub struct ServiceInit {
    /// Ambient facts.
    pub env: EnvFacts,
    /// `--config-dir`.
    pub config_dir: Option<PathBuf>,
    /// Worker threads (None = available parallelism, min 1).
    pub threads: Option<NonZeroUsize>,
    /// Standard input with the `--stdin-name` hint, read by the service at most once, on the first `-` spec it
    /// parses (§1.8.7 item 7; was an eagerly read `Option<Blob>`).
    pub stdin: Option<StdinSource>,
    /// Clock.
    pub clock: Arc<dyn Clock>,
    /// External conversion runner (`StdProcessRunner` in adapters, fakes in tests).
    pub runner: Arc<dyn ProcessRunner>,
}

/// Operation context for exit-code mapping (§2.21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiOp { Quick, TextCompare, FolderCompare, Format, Vfs, GuiOpen { wait: bool }, GuiControl, Other }

/// The service. `Send + Sync`; cheap to clone (`Arc` inside); one per process, shared by all CLI commands,
/// GUI views and jobs.
#[derive(Clone)]
pub struct Service { /* Arc<{ EnvFacts, ConfigLocation, ProgramOptions, Arc<FormatRegistry>, Arc<VfsRouter>,
                               Arc<rayon::ThreadPool>, Arc<dyn Clock>, Arc<dyn ProcessRunner>, JobIdAllocator,
                               startup warnings }> */ }

impl Service {
    /// Resolves config, loads options and formats, builds the VFS router and the thread pool
    /// (threads named `hikasa-worker-N`).
    pub fn new(init: ServiceInit) -> Result<Service, ApiError>;
    /// Configuration folder.
    pub fn config(&self) -> &ConfigLocation;
    /// Program options.
    pub fn options(&self) -> &ProgramOptions;
    /// Format registry.
    pub fn registry(&self) -> &Arc<FormatRegistry>;
    /// Warnings collected during construction (unknown config sections, skipped user formats).
    pub fn startup_warnings(&self) -> &[Warning];
    /// Option keys ignored while loading `options.toml` (logged with `--verbose`, D-31).
    pub fn ignored_option_keys(&self) -> &[String];
    /// Spec context from the captured facts.
    pub fn spec_context(&self) -> SpecContext;
    /// Parses a user spec.
    pub fn parse_spec(&self, input: &OsStr) -> Result<Spec, ApiError>;
    /// The IPC endpoint of this user and configuration (§2.23).
    pub fn endpoint(&self) -> Result<Endpoint, ApiError>;
    /// Creates a job control for a direct (blocking) call.
    pub fn job(&self, kind: JobKind, progress: Arc<dyn ProgressSink>) -> JobControl;
    /// Runs `f` on a new thread named `hikasa-job-<id>` with a fresh `CancelToken`, a lossy progress channel
    /// and the panic boundary (`run_guarded`). The GUI's way to start every long operation. The body runs on that
    /// thread, not inside the pool (§1.8.7 item 10).
    pub fn spawn<T: Send + 'static>(
        &self,
        kind: JobKind,
        title: String,
        f: impl FnOnce(&Service, &JobControl) -> Result<T, ApiError> + Send + 'static,
    ) -> JobHandle<T>;

    /// Quick compare (§3.1).
    pub fn quick(&self, req: &QuickRequest, job: &JobControl) -> Result<QuickResponse, ApiError>;
    /// Text compare (§3.2). `observer` receives `loaded`, phases and partial runs (GUI first screen).
    pub fn text_compare(&self, req: &TextCompareRequest, job: &JobControl,
                        observer: Option<&mut dyn CompareObserver>) -> Result<TextCompareResponse, ApiError>;
    /// Folder compare, step 1 (no I/O on the bases): parses specs and rules and creates the live model, which
    /// the GUI can render before and during the scan.
    pub fn folder_prepare(&self, req: &FolderCompareRequest) -> Result<PreparedFolderCompare, ApiError>;
    /// Folder compare, step 2: scans into the prepared model (§3.3).
    pub fn folder_run(&self, prepared: &PreparedFolderCompare, job: &JobControl, observer: &dyn ScanObserver)
        -> Result<FolderCompareResponse, ApiError>;
    /// `folder_prepare` + `folder_run` (CLI).
    pub fn folder_compare(&self, req: &FolderCompareRequest, job: &JobControl, observer: &dyn ScanObserver)
        -> Result<FolderCompareResponse, ApiError>;
    /// Renders a text report with `ReportMeta::for_text(…, Some(clock.now()), "hikasa <version>")`.
    pub fn render_text(&self, resp: &TextCompareResponse, opts: &TextReportOptions, out: &mut dyn Write)
        -> Result<(), ApiError>;
    /// Renders a folder report with `ReportMeta::for_folder(…)`.
    pub fn render_folder(&self, resp: &FolderCompareResponse, opts: &FolderReportOptions, out: &mut dyn Write)
        -> Result<(), ApiError>;
    /// Writes report output to a spec atomically (`--output`).
    pub fn write_output(&self, target: &OsStr, bytes: &[u8]) -> Result<Spec, ApiError>;

    /// `format list`.
    pub fn format_list(&self) -> dto::FormatListDto;
    /// `format show`.
    pub fn format_show(&self, id_or_name: &str) -> Result<dto::FormatShowDto, ApiError>;
    /// `format detect` (a "-" file uses the stdin name hint, D-15).
    pub fn format_detect(&self, file: &OsStr) -> Result<dto::FormatDetectDto, ApiError>;
    /// `format tokens`.
    pub fn format_tokens(&self, req: &FormatTokensRequest, job: &JobControl) -> Result<dto::FormatTokensDto, ApiError>;

    /// `vfs ls`.
    pub fn vfs_ls(&self, spec: &OsStr, follow: FollowLinks, job: &JobControl) -> Result<dto::VfsListDto, ApiError>;
    /// `vfs stat` (returns the §4.3.4 shape, §1.8.7 item 1).
    pub fn vfs_stat(&self, spec: &OsStr, follow: FollowLinks) -> Result<dto::VfsStatDto, ApiError>;
    /// `vfs cat`: copies content into `out`; returns bytes copied.
    pub fn vfs_cat(&self, spec: &OsStr, out: &mut dyn Write, job: &JobControl) -> Result<u64, ApiError>;

    /// `gui open` and `--gui` (§3.6). `launcher = None` means `--no-launch`.
    pub fn gui_open(&self, req: &GuiOpenRequest, launcher: Option<&dyn GuiLauncher>) -> Result<GuiOpenOutcome, ApiError>;
    /// `gui ping` (never launches, D-22).
    pub fn gui_ping(&self) -> Result<ServerInfo, ApiError>;
    /// `gui list`.
    pub fn gui_list(&self) -> Result<Vec<ViewInfo>, ApiError>;
    /// `gui close`.
    pub fn gui_close(&self, view: ViewId) -> Result<(), ApiError>;
    /// `gui quit`.
    pub fn gui_quit(&self) -> Result<(), ApiError>;
}

/// Quick compare method (`--size|--crc|--binary|--rules`; default Rules, CLI-003 `--qc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickMethod { Size, Crc, Binary, #[default] Rules }

/// `hikasa quick`.
#[derive(Debug, Clone, Default)]
pub struct QuickRequest {
    pub left: OsString,
    pub right: OsString,
    pub method: QuickMethod,
    pub ignore_unimportant: bool,
    pub format: Option<String>,
    pub rules_file: Option<OsString>,
    pub rules_patch: TextRulesPatch,
}

/// The effective format of a file comparison (D-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatChoice {
    /// Id (`None` = Everything Else).
    pub id: Option<FormatId>,
    /// Display name.
    pub name: String,
    /// Kind.
    pub kind: FormatKind,
    /// Why it was chosen.
    pub reason: DetectReason,
}

/// Result of `quick`.
#[derive(Debug, Clone)]
pub struct QuickResponse {
    pub left: Spec,
    pub right: Spec,
    pub method: QuickMethod,
    /// Effective format (`None` for the size, CRC and binary methods).
    pub format: Option<FormatChoice>,
    pub outcome: ContentOutcome,
    pub ignore_unimportant: bool,   // §1.8.7 item 17
    pub exit_code: ExitCode,
}

/// `hikasa text compare`.
#[derive(Debug, Clone, Default)]
pub struct TextCompareRequest {
    pub left: OsString,
    pub right: OsString,
    pub format: Option<String>,
    /// Per-side encoding overrides (TC-001): `--encoding` fills both, `--left-encoding`/`--right-encoding` one.
    pub encodings: Pair<Option<String>>,
    pub rules_file: Option<OsString>,
    pub rules_patch: TextRulesPatch,
    pub ignore_unimportant: bool,
}

/// Result of `text compare`.
#[derive(Debug, Clone)]
pub struct TextCompareResponse {
    pub comparison: Arc<TextComparison>,
    /// Effective format (D-15).
    pub format: FormatChoice,
    /// Detection per side by name (`None` for an unnamed side); informational (`format_mismatch`).
    pub detection: Pair<Option<DetectReason>>,
    pub verdict: ContentResult,
    pub ignore_unimportant: bool,   // §1.8.7 item 17
    pub exit_code: ExitCode,
    pub warnings: Vec<Warning>,
}

/// `hikasa folder compare`.
#[derive(Debug, Clone, Default)]
pub struct FolderCompareRequest {
    pub left: OsString,
    pub right: OsString,
    pub rules_file: Option<OsString>,
    /// `--criteria` (D-57), applied after the rules file and before `patch`.
    pub criteria: Option<String>,
    pub patch: FolderRulesPatch,
    /// Rules for `--content rules` (the injected RulesComparator).
    pub text_rules_file: Option<OsString>,
    pub text_patch: TextRulesPatch,
    pub ignore_unimportant: bool,
}

/// A prepared folder comparison (`Service::folder_prepare`).
#[derive(Clone)]
pub struct PreparedFolderCompare { /* left, right: Spec, rules: FolderRules, text_rules: Option<TextRules>,
                                      ignore_unimportant, model: Arc<FolderModel> */ }

impl PreparedFolderCompare {
    /// The live model (empty until `folder_run` starts filling it).
    pub fn model(&self) -> &Arc<FolderModel>;
    /// Effective folder rules.
    pub fn rules(&self) -> &FolderRules;
}

/// Result of `folder compare`.
#[derive(Debug, Clone)]
pub struct FolderCompareResponse {
    pub comparison: FolderComparison,
    pub verdict: CompareVerdict,
    /// The effective ignore-unimportant (request flag or criteria word; JSON and reports use it, §1.8.7 item 17).
    pub ignore_unimportant: bool,
    pub exit_code: ExitCode,
    pub warnings: Vec<Warning>,
}

/// `hikasa format tokens`.
#[derive(Debug, Clone, Default)]
pub struct FormatTokensRequest {
    pub file: OsString,
    pub format: Option<String>,
    pub encoding: Option<String>,
    /// 1-based inclusive line range filter for output (all lines are still tokenised for state).
    pub lines: Option<RangeInclusive<u32>>,
}

/// API errors. `exit_code(op)` implements §2.21.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    /// Bad argument combination or value (also unknown `gui open` switches and vocabulary words).
    #[error("{0}")]
    InvalidArgument(String),
    /// Spec syntax.
    #[error(transparent)]
    Spec(#[from] SpecError),
    /// An input could not be loaded (stat/read/unsupported scheme/too large).
    #[error("{side} input {spec}: {source}")]
    LoadFailed { side: Side, spec: String, #[source] source: VfsError },
    /// Other VFS failure.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// Format failure.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// Text engine failure.
    #[error(transparent)]
    Text(#[from] TextError),
    /// Folder engine failure.
    #[error(transparent)]
    Folder(#[from] FolderError),
    /// Report failure.
    #[error(transparent)]
    Report(#[from] ReportError),
    /// Configuration failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// GUI connection failure.
    #[error(transparent)]
    Ipc(#[from] IpcError),
    /// The GUI executable was not found (exit 103).
    #[error("cannot find the hikasa-gui executable")]
    GuiNotFound,
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
    /// A panic or invariant violation (bug).
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    /// Error kind (JSON `error.kind`).
    pub fn kind(&self) -> ErrorKind;
    /// Exit code for this error in the context of `op` (§2.21).
    pub fn exit_code(&self, op: ApiOp) -> ExitCode;
    /// Serialisable report.
    pub fn report(&self) -> ErrorReport;
}
```

#### 1.8.5 GUI client helpers and the shared `gui open` parser (`hikasa_session::api::gui`)

```rust
/// Starts the GUI process and sleeps between connection attempts (implemented by hikasa-cli with
/// `std::process::Command`, detached, stdio null, and `std::thread::sleep`; tests record the calls).
pub trait GuiLauncher {
    /// Starts the GUI executable (`find_gui_exe`) with `launch.args` and `launch.env`, which
    /// `Service::gui_launch` chooses so that the GUI serves the CLI's endpoint (`--config-dir` of an explicit or
    /// portable configuration, `HIKASA_IPC_ENDPOINT`; §1.8.7 item 18). Requests then arrive over IPC.
    fn launch(&self, launch: &GuiLaunch) -> Result<(), LaunchError>;
    /// Sleeps `d` (the retry budget is the sum of these durations; core never reads a clock, T1).
    fn sleep(&self, d: Duration);
}

/// Launch failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LaunchError {
    /// No candidate executable exists (exit 103).
    #[error("hikasa-gui executable not found")]
    NotFound,
    /// Spawning failed.
    #[error("cannot start hikasa-gui: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Candidate GUI executables, in order: sibling of the CLI executable, then each `PATH` entry (pure).
pub fn gui_exe_candidates(exe_dir: Option<&Path>, path_var: Option<&OsStr>, platform: Platform) -> Vec<PathBuf>;
/// `$HIKASA_GUI_EXE` alone when set, else the first existing candidate (§1.8.7 item 18).
pub fn find_gui_exe(facts: &EnvFacts) -> Option<PathBuf>;

/// `gui open` request (also built by `--gui`, D-50).
#[derive(Debug, Clone, Default)]
pub struct GuiOpenRequest {
    /// Raw specs (resolved to absolute canonical specs before sending: the GUI has another cwd; "-" is spooled,
    /// `clipboard://` is rejected, D-49).
    pub specs: Vec<OsString>,
    /// Forced view kind (`--fv`); None → inferred (CLI-002).
    pub kind: Option<ViewKind>,
    /// View options.
    pub options: OpenOptions,
    /// Wait for the view to close (CLI-G01).
    pub wait: bool,
    /// Rules files as typed (`--gui`), made absolute by `resolve_open` (§1.8.7 item 20).
    pub text_rules_file: Option<OsString>,
    pub folder_rules_file: Option<OsString>,
}

/// `gui open` outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiOpenOutcome {
    /// Opened, not waiting.
    Opened { view: ViewId },
    /// Waited until the view closed; `exit_code` is the raw code sent by the GUI (D-23).
    Closed { view: ViewId, result: ViewResult, exit_code: u8 },
}

/// Parsed `hikasa gui open` / `hikasa-gui` arguments (CLI-001…CLI-003, D-51).
#[derive(Debug, Clone, Default)]
pub struct OpenArgs {
    /// The request.
    pub request: GuiOpenRequest,
    /// `--no-launch` (CLI only).
    pub no_launch: bool,
    /// `--solo` (GUI only: force a new instance).
    pub solo: bool,
    /// `--json`, `--quiet`, `--no-color` given after `open` (output flags only).
    pub json: bool,
    pub quiet: bool,
    pub no_color: bool,
    /// `--config-dir`, `--log`, `--verbose` (the GUI's configuration; the CLI merges them with its global flags
    /// before it builds the service, §1.8.7 item 34).
    pub config_dir: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub verbose: bool,
    /// `switch_ignored` warnings for recognised CLI-003 switches that M1 does not implement (they are
    /// forwarded in `OpenOptions::extra`).
    pub warnings: Vec<Warning>,
    /// `--stdin-name`, `--help` (§1.8.7 item 5).
    pub stdin_name: Option<String>,
    pub help: bool,
}

/// Rewrites CLI-001 spellings to `--name[=value]`: `-name[=v]` for every known switch, `/name[=v]` on Windows
/// only (on Unix a leading `/` is a path), and aliases (`--readonly` → `--ro`, `--lefttitle` → `--title1`,
/// `--fileviewer` → `--fv`, `--ignoreunimportant` → `--iu`, `--quickcompare` → `--qc`, …). Pure, table-driven.
pub fn normalize_legacy_args(args: &[OsString], platform: Platform) -> Vec<OsString>;

/// Parses normalised arguments: positional specs (CLI-002) and the switches of §1.9.2's `gui open` row. A value
/// is given as `--name=value` or `--name value`. Unknown switch names are `InvalidArgument` (exit 100).
pub fn parse_open_args(args: &[OsString], platform: Platform) -> Result<OpenArgs, ApiError>;

/// CLI-002 positional rules for M1: two folders → FolderCompare; two files → by the effective format kind
/// (Text/Table → TextCompare; other kinds → TextCompare with warning until their views exist); one spec or more
/// than two specs → `InvalidArgument` (M2 adds sessions, workspaces, merge, patch, snapshot).
pub fn infer_view_kind(service: &Service, specs: &[Spec], forced: Option<ViewKind>) -> Result<ViewKind, ApiError>;
```

#### 1.8.6 IPC (`hikasa_session::ipc`, ARCH-090…092)

```rust
/// Protocol version spoken by this build.
pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum frame payload (bytes). Larger frames are a protocol error.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Writes one frame: u32 little-endian payload length, then the UTF-8 JSON payload.
pub fn write_frame(w: &mut dyn Write, payload: &[u8]) -> Result<(), IpcError>;
/// Reads one frame. `Ok(None)` = clean EOF at a frame boundary. EOF inside a frame = `ConnectionLost`.
pub fn read_frame(r: &mut dyn Read, max: u32) -> Result<Option<Vec<u8>>, IpcError>;
/// Serialises an envelope (field order: `id`, `type`, body fields) and writes it.
pub fn send<T: Serialize>(w: &mut dyn Write, msg: &Envelope<T>) -> Result<(), IpcError>;
/// Reads a request with the two-phase decode of §4.4.3 (D-46): `Ok(None)` at clean EOF; an envelope whose body
/// does not decode gives `Err(IpcError::BadMessage { id, .. })` so that the caller can answer with that id.
pub fn recv_request(r: &mut dyn Read) -> Result<Option<Envelope<Request>>, IpcError>;
/// Reads a response (same two-phase decode; unknown `type` values become `Response::Unknown`).
pub fn recv_response(r: &mut dyn Read) -> Result<Option<Envelope<Response>>, IpcError>;

/// Transport address of an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EndpointAddress {
    /// Unix domain socket path.
    UnixSocket(PathBuf),
    /// Windows named pipe name (`hikasa-<user>[-<h8>]`).
    NamedPipe(String),
}

/// Per-user, per-configuration endpoint (§2.23, D-47).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Address.
    pub address: EndpointAddress,
    /// Single-instance lock file (Unix: `<socket>.lock`; Windows: `<temp>\<pipe name>.lock`).
    pub lock_path: PathBuf,
    /// Hikasa's own socket folder, when the endpoint uses it (§1.8.7 item 2).
    pub own_dir: Option<PathBuf>,
    /// The user that must own the socket folder (Unix; `EnvFacts::uid`; §1.8.7 item 11).
    pub uid: Option<u32>,
}

impl Endpoint {
    /// Computes the endpoint (§2.23). Pure: `ipc_endpoint_override` wins; nothing is created here (the server
    /// creates and verifies the socket folder in `IpcServer::bind`).
    pub fn for_config(facts: &EnvFacts, config: &ConfigLocation) -> Result<Endpoint, IpcError>;
    /// Display text for messages.
    pub fn display(&self) -> String;
    /// Verifies the socket folder before anything talks through it (§2.23, §1.8.7 item 11).
    pub fn verify_folder(&self) -> Result<(), IpcError>;
}

/// FNV-1a 64-bit over bytes (stable everywhere; endpoint suffixes).
pub const fn fnv1a64(bytes: &[u8]) -> u64;

/// Single-instance lock (ARCH-090): an exclusive `File::try_lock` on `Endpoint::lock_path`, held for the
/// server's lifetime. Race-free, unlike probing the socket.
#[derive(Debug)]
pub struct InstanceLock { /* File */ }

impl InstanceLock {
    /// `Ok(None)` when another live instance holds the lock.
    pub fn try_acquire(endpoint: &Endpoint) -> Result<Option<InstanceLock>, IpcError>;
}

/// Server-side request handler (implemented by hikasa-gui).
pub trait IpcHandler: Send + Sync + 'static {
    /// Identity for the `Hello` reply.
    fn server_info(&self) -> ServerInfo;
    /// Handles one request after the handshake (`Request::Unknown` never reaches it: the server answers
    /// `unsupported_request` itself). For `OpenView { wait: true }`, `waiter` is `Some`; the handler stores it and
    /// later calls `ViewWaiter::view_closed`. The return value is the immediate response. Must not block: the
    /// GUI posts to its event loop and returns.
    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response;
}

/// Sends the deferred `ViewClosed` on the originating connection (no-op when the client is gone). Dropped without
/// `view_closed`, it sends `ViewClosed` with reason `abandoned` and exit code 100 (§1.8.7 item 12).
#[derive(Debug)]
pub struct ViewWaiter { /* connection writer queue, gate, request id */ }

impl ViewWaiter {
    /// Queues `ViewClosed` (exactly once: consumes self; never blocks, §1.8.7 item 13).
    pub fn view_closed(self, view: ViewId, result: ViewResult, exit_code: ExitCode);
}

/// A bound IPC server.
#[derive(Debug)]
pub struct IpcServer { /* listener, endpoint, InstanceLock */ }

impl IpcServer {
    /// Acquires the instance lock (`AlreadyRunning` when held). Unix: creates the socket folder with mode 0700
    /// when it is Hikasa's own fallback folder and verifies its owner and mode (`InsecureEndpoint` otherwise),
    /// removes a stale socket file, binds, and sets the socket mode to 0600.
    pub fn bind(endpoint: &Endpoint) -> Result<IpcServer, IpcError>;
    /// Starts the accept thread (one thread per connection) and returns its handle. Unix: rejects peers whose
    /// `peer_creds().euid()` differs from the owner uid of the socket file.
    pub fn spawn(self, handler: Arc<dyn IpcHandler>) -> Result<ServerHandle, IpcError>;
}

/// A running server.
#[derive(Debug)]
pub struct ServerHandle { /* stop flag, endpoint, accept thread */ }

impl ServerHandle {
    /// Stops accepting: sets the stop flag, wakes the blocked `accept` by connecting to its own endpoint once
    /// (self-connect; no sleeping or polling), and joins the accept thread; then sends every outstanding waiter
    /// `ViewClosed` (reason `shutdown`, exit 100) and closes every connection after its last frame (§1.8.7 item 12).
    /// The GUI notifies its waiters with the views' real codes before it calls this.
    pub fn shutdown(self) -> Result<(), IpcError>;
}

/// Retry policy for connecting after launching the GUI. The budget is the sum of the slept durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// First delay (default 50 ms), doubled after each attempt up to `max_delay` (default 500 ms).
    pub initial_delay: Duration,
    /// Maximum delay.
    pub max_delay: Duration,
    /// Total budget of slept time (default 10 s); exceeded → `IpcError::LaunchTimeout`.
    pub total: Duration,
}
impl Default for RetryPolicy { /* 50 ms, 500 ms, 10 s */ }

/// IPC client.
#[derive(Debug)]
pub struct IpcClient { /* stream, next request id, ServerInfo */ }

impl IpcClient {
    /// Verifies the socket folder, connects and performs the `Hello` handshake. Every call has one deadline of
    /// `timeout` for its immediate response (expiry → `IpcError::Timeout`; §1.8.7 item 14).
    pub fn connect(endpoint: &Endpoint, client: &str, timeout: Duration) -> Result<IpcClient, IpcError>;
    /// Negotiated server identity.
    pub fn server(&self) -> &ServerInfo;
    /// Sends a request and returns its immediate response and the request id used.
    pub fn call(&mut self, request: Request) -> Result<(u64, Response), IpcError>;
    /// Blocks (receive timeout disabled) until `ViewClosed` for the `OpenView` request `request_id` arrives;
    /// EOF → `ConnectionLost`. Returns the raw exit code byte.
    pub fn wait_view_closed(&mut self, request_id: u64) -> Result<(ViewId, ViewResult, u8), IpcError>;
}

/// Connects; if there is no GUI and `launcher` is given, launches it once and retries per `policy`, sleeping
/// through `GuiLauncher::sleep`. Errors: no GUI and no launcher → `Ipc(NotRunning)`; launcher reports
/// `NotFound` → `GuiNotFound`; budget exhausted → `Ipc(LaunchTimeout)`.
pub fn connect_or_launch(
    endpoint: &Endpoint,
    client: &str,
    launcher: Option<&dyn GuiLauncher>,
    launch: &GuiLaunch,            // §1.8.7 item 18
    policy: &RetryPolicy,
    timeout: Duration,
) -> Result<IpcClient, ApiError>;

/// IPC errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IpcError {
    /// No server listening.
    #[error("the Hikasa GUI is not running")]
    NotRunning,
    /// Another instance holds the endpoint lock.
    #[error("another Hikasa GUI instance is running")]
    AlreadyRunning,
    /// The socket folder has the wrong owner or permissions.
    #[error("insecure IPC endpoint {path}: {reason}")]
    InsecureEndpoint { path: String, reason: &'static str },
    /// Frame larger than the limit.
    #[error("frame of {len} bytes exceeds the limit of {max}")]
    FrameTooLarge { len: u32, max: u32 },
    /// Malformed frame or sequence (no usable request id).
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A frame with a valid `id` whose body does not decode.
    #[error("bad message {id}: {message}")]
    BadMessage { id: u64, message: String },
    /// No common protocol version.
    #[error("incompatible protocol versions (client {client}, server {server})")]
    IncompatibleVersion { client: u32, server: u32 },
    /// The peer answered with an error (`code` is the wire string, possibly unknown to this build).
    #[error("{code}: {message}")]
    Remote { code: String, message: String },
    /// EOF while waiting for a response or `ViewClosed`.
    #[error("connection to the GUI was lost")]
    ConnectionLost,
    /// The launched GUI did not become reachable within the retry budget.
    #[error("the Hikasa GUI did not start listening in time")]
    LaunchTimeout,
    /// An immediate response did not arrive within the timeout.
    #[error("the Hikasa GUI did not answer in time")]
    Timeout,
    /// Transport I/O.
    #[error("ipc I/O: {0}")]
    Io(#[from] std::io::Error),
}
```

The protocol types (`Envelope`, `Request`, `Response`, `ViewKind`, `ViewId`, `OpenOptions`, `ReadOnly`,
`ViewResult`, `CloseReason`, `ServerInfo`, `ViewInfo`, `IpcErrorCode`) are specified in §4.4.

#### 1.8.7 Accepted additions and deviations (wave 4)

The wave-4 review of hikasa-session (conformance, consumer and adversarial findings; maintainer decisions 1–10)
accepted the following public items, deviations and amendments. They are part of the contract: hikasa-cli
(wave 5) and the GUI (wave 6) build on them, and where an item here refines the text of §1.8.1–§1.8.6 the item
wins. API changes were allowed because no adapter existed yet; the breaking ones are listed first. Amended
normative text: §1.8.2–§1.8.6, §1.9.2, §1.10, §2.21, §2.22, §2.23, §3.5, §3.6, §3.7, §4.2, §4.3.4, §4.4.2,
§4.4.3, §5.1, §5.2, §5.6, §6.1, §6.2.8, §6.3 and Appendix B (INV-IPC-2); upstream, §1.6.8 item 24
(`FolderRulesPatch` unix fields) and §1.7.3, session-review additions (public escapers, `format_mismatch` and the header).

**Breaking changes** (all made before any adapter existed):

| Item | Before | Now |
|---|---|---|
| `ServiceInit::stdin` | `Option<Blob>`, read by the adapter when a spec is `-` | `Option<StdinSource>`: a name hint and a read function, called at most once, by the first `-` spec the service parses |
| `GuiLauncher::launch` | `fn launch(&self)` ("without arguments") | `fn launch(&self, launch: &GuiLaunch)`: arguments and environment chosen by the session |
| `connect_or_launch` | `(endpoint, client, launcher, policy, timeout)` | `(endpoint, client, launcher, launch: &GuiLaunch, policy, timeout)` |
| `Endpoint` | `address`, `lock_path`, `own_dir` | plus `uid: Option<u32>`, the user that must own the socket folder |
| `EnvFacts` | … `path_var` | plus `gui_exe_override: Option<PathBuf>` (`$HIKASA_GUI_EXE`) and `no_color: bool` (`$NO_COLOR` non-empty); `EnvFacts::new` fills both |
| `GuiOpenRequest` | `specs`, `kind`, `options`, `wait` | plus `text_rules_file`, `folder_rules_file: Option<OsString>` (as typed) |
| `OpenOptions` (wire) | §4.4.2 | plus `encodings`, `display`, `context_lines`, `folders`, `suppress_filters` (absent → defaults, ignored by older peers) |
| `QuickResponse`, `TextCompareResponse`, `FolderCompareResponse` | — | plus `ignore_unimportant: bool` (the effective flag) |
| `QuickDto::new` | `(resp, ignore_unimportant)` | `(resp)` |
| `SCHEMAS` | `[&str; 16]` | `[&str; 18]` (`hikasa.completions.v1`, `hikasa.usage_error.v1`) |
| `CloseReason` | `User`, `Shutdown`, `LoadFailed` | plus `Abandoned` (`ALL: [CloseReason; 4]`); travels as a string, so older peers accept it |
| `IpcServer::set_peer_uid` (doc-hidden test hook) | `Option<u32>` (`None` disabled the check) | `u32`: replaces the expected uid, never disables the check |
| `JobHandle::wait` after `try_result` returned the result | `Internal` | `InvalidArgument("… already taken …")` |
| `Service::spawn` | body inside `pool.install` | body on its own `hikasa-job-<id>` thread (item 11) |
| cache fallback (§2.22) | `<temp>/hikasa-cache` | `<temp>/hikasa-cache-<id>` (id as for the socket folder) |

**Additive API** (complete list beyond §1.8.1–§1.8.6 and the table above):

```rust
// lib.rs
pub use api::StdinSource;

// host.rs (see the table for the two new EnvFacts fields)

// error.rs
pub enum SessionError { /* … */
    /// A configuration file of a newer version (D-31, SES-052); kind `unsupported`.
    Schema { path: String, source: SchemaError },
    /// `--config-dir ""` (an unset shell variable) is refused, never the current directory; kind `invalid_input`.
    EmptyConfigDir,
}

// atomic.rs
pub const TEMP_NAME_PREFIX_MAX: usize = 200;   // the target name kept in `.<name>.hikasa-tmp-<n>`

// jobs.rs
impl<T> JobHandle<T> { pub fn is_finished(&self) -> bool; }

// api/mod.rs
pub struct StdinSource { pub name: Option<String>, /* read: Box<dyn FnOnce() -> io::Result<Vec<u8>> + Send> */ }
impl StdinSource {
    pub fn new(name: Option<String>, read: impl FnOnce() -> io::Result<Vec<u8>> + Send + 'static) -> StdinSource;
    pub fn from_bytes(bytes: impl Into<Vec<u8>>, name: Option<String>) -> StdinSource;
}
impl ServiceInit { pub fn new(env: EnvFacts, clock: Arc<dyn Clock>, runner: Arc<dyn ProcessRunner>) -> ServiceInit; }
pub const HIKASA_VERSION: &str;              // CARGO_PKG_VERSION
pub const MAX_STREAM_THREADS: usize = 4;     // min(4, pool size), §5.1
impl Service {
    pub fn env(&self) -> &EnvFacts;
    pub fn vfs(&self) -> &VfsRouter;          // with standard input once a `-` spec was parsed
    pub fn pool(&self) -> &rayon::ThreadPool; // has a panic handler (item 19)
    pub fn clock(&self) -> &Arc<dyn Clock>;
    pub fn stream_options(&self) -> StreamOptions;
    pub fn generator(&self) -> String;        // "hikasa <version>"
    pub fn stdin_name(&self) -> Option<&str>; // the --stdin-name hint, without reading stdin
    pub fn gui_launch(&self) -> GuiLaunch;
    pub fn resolve_open(&self, req: &GuiOpenRequest) -> Result<ResolvedOpen, ApiError>;
    pub fn check_open(&self, resolved: &ResolvedOpen) -> Result<(), ApiError>;
    pub fn open_view_request(&self, kind: ViewKind, specs: &[String], options: &OpenOptions)
        -> Result<ViewRequest, ApiError>;
    pub fn forward_open(&self, req: &GuiOpenRequest) -> Result<GuiOpenReport, ApiError>;
    pub fn gui_open_with(&self, req: &GuiOpenRequest, launcher: Option<&dyn GuiLauncher>, policy: &RetryPolicy)
        -> Result<GuiOpenReport, ApiError>;
    pub fn gui_ping_dto(&self) -> Result<dto::GuiPingDto, ApiError>;
    pub fn vfs_cat_dto(&self, spec: &OsStr, job: &JobControl) -> Result<dto::VfsCatDto, ApiError>;
    pub fn format_toml(&self, id_or_name: &str) -> Result<String, ApiError>;   // human `format show`
    pub fn render_text_with(&self, resp: &TextCompareResponse, opts: &TextReportOptions, settings: &ReportSettings,
                            out: &mut dyn Write) -> Result<Vec<Warning>, ApiError>;
    pub fn render_folder_with(&self, resp: &FolderCompareResponse, opts: &FolderReportOptions,
                              settings: &ReportSettings, out: &mut dyn Write) -> Result<(), ApiError>;
}
pub struct ReportSettings { pub title: Option<String>, pub style: HtmlStyle, pub list_summary: bool }  // Default: list_summary = true

// api/requests.rs
impl QuickMethod { pub const fn as_str(self) -> &'static str; pub fn parse(word: &str) -> Option<QuickMethod>; }
impl TextCompareRequest { pub fn for_specs(left: &Spec, right: &Spec) -> TextCompareRequest; }   // FC-062
pub fn spec_os_string(spec: &Spec) -> OsString;   // exact OS path for local specs, canonical otherwise

// api/responses.rs
impl FormatChoice { pub fn json(&self) -> FormatJson; }
impl TextCompareResponse { pub fn json(&self, view: &ViewOptions) -> TextCompareJson; }       // uses the response's iu
impl FolderCompareResponse { pub fn json(&self, display: &DisplayOptions) -> FolderCompareJson; } // uses the effective iu
impl PreparedFolderCompare { pub fn specs(&self) -> Pair<&Spec>; pub fn text_rules(&self) -> Option<&TextRules>;
                             pub fn ignore_unimportant(&self) -> bool; pub fn warnings(&self) -> &[Warning]; }

// api/error.rs
pub enum ApiError { /* … */
    /// A rules file that does not parse or validate, named by its canonical spec (JSON `error.spec`); kind and
    /// exit code are those of `source`.
    RulesFile { spec: String, source: Box<ApiError> },
}
impl ApiError { pub fn is_broken_pipe(&self) -> bool; }

// api/vocab.rs (also re-exported from api)
pub fn parse_folder_mode(word: &str) -> Result<FolderMode, ApiError>;     // --folders
pub fn parse_content_method(word: &str) -> Result<ContentMethod, ApiError>; // --content none|crc|binary|rules
pub fn parse_algorithm(word: &str) -> Result<AlgorithmChoice, ApiError>;  // --algorithm, with the valid words
pub fn parse_line_range(text: &str) -> Result<RangeInclusive<u32>, ApiError>; // --lines a-b | n
pub fn resolve_text_display(display: Option<&str>, context: Option<u32>, default_context: u32, json: bool)
    -> Result<DisplayMode, ApiError>;
pub fn resolve_folder_display(display: Option<&str>, folders: Option<&str>, suppress_filters: bool,
    ignore_unimportant: bool) -> Result<DisplayOptions, ApiError>;
pub const DEFAULT_FOLDER_FILTER: FolderDisplayFilter = FolderDisplayFilter::Differences;
pub const DEFAULT_FOLDER_MODE: FolderMode = FolderMode::CompareStructure;
pub use hikasa_report::folder_mode_word;

// api/human.rs: the human lines of the list commands (§1.9.2 amendment)
pub fn token_lines(dto: &FormatTokensDto) -> Vec<String>;   // L<line>:<start>-<end>\t<element>\t<escaped text>
pub fn vfs_ls_line(entry: &VfsEntryDto) -> String;          // <kind><attrs> <size> <mtime> <escaped name>[ -> target]
pub fn vfs_stat_lines(dto: &VfsStatDto) -> Vec<String>;     // the ls line, then `capabilities: …`
pub fn gui_list_line(view: &ViewInfo) -> String;            // <view>\t<kind>\t<title>\t<spec>…
pub fn format_list_line(item: &FormatListItemDto) -> String;// <position>\t<id>\t<status>\t<name>\t<mask>
pub fn format_detect_line(dto: &FormatDetectDto) -> String; // <id or ->\t<name>

// api/dto.rs
pub const COMPLETIONS_SCHEMA: &str = "hikasa.completions.v1";   // { "shell", "script" }
pub const USAGE_ERROR_SCHEMA: &str = "hikasa.usage_error.v1";   // result null, error.kind invalid_input, exit 100
pub struct CompletionsDto { pub shell: String, pub script: String }
impl<T: Serialize> JsonEnvelope<T> {
    pub fn success_raw(schema: &'static str, exit_code: u8, result: T, warnings: Vec<Warning>) -> JsonEnvelope<T>;
}
impl JsonEnvelope<()> { pub fn usage_error(message: &str) -> JsonEnvelope<()>; }
pub fn write_json<T: Serialize + ?Sized>(out: &mut dyn Write, value: &T) -> io::Result<()>;
pub fn base64_encode(bytes: &[u8]) -> String;
// plus the DTO types of §4.3.4 (FormatListDto … VersionDto) with `new`/`current` constructors

// api/gui.rs (also re-exported from api)
pub struct GuiLaunch { pub args: Vec<OsString>, pub env: Vec<(OsString, OsString)> }
pub fn find_gui_exe(facts: &EnvFacts) -> Option<PathBuf>;   // $HIKASA_GUI_EXE alone, else the first existing candidate
pub struct ViewWords { pub display: Option<String>, pub context_lines: Option<u32>, pub folders: Option<String>,
                       pub suppress_filters: bool, pub expand_all: bool }
impl GuiOpenRequest {
    pub fn for_text(req: &TextCompareRequest, view: &ViewWords, wait: bool) -> GuiOpenRequest;
    pub fn for_folder(req: &FolderCompareRequest, view: &ViewWords, wait: bool) -> GuiOpenRequest;
}
impl GuiOpenOutcome { pub fn exit_code(&self) -> u8; }   // Opened → 0, Closed → the raw code
pub struct GuiOpenReport { pub outcome: GuiOpenOutcome, pub kind: ViewKind, pub specs: Vec<String>,
                           pub spooled_stdin: Option<PathBuf> }
impl GuiOpenReport { pub fn dto(&self) -> dto::GuiOpenDto; pub fn exit_code(&self) -> u8; }
pub struct ResolvedOpen { pub kind: ViewKind, pub specs: Vec<String>, pub options: OpenOptions,
                          pub spooled_stdin: Option<PathBuf> }
pub enum ViewRequest {
    Text { request: TextCompareRequest, view: ViewOptions },
    Folder { request: FolderCompareRequest, display: DisplayOptions, expand_all: bool },
}
pub enum ViewOutcome { NoVerdict, LoadFailed, Text { result: ContentResult, ignore_unimportant: bool },
                       Folder { verdict: CompareVerdict } }
impl ViewOutcome { pub fn close(&self, reason: CloseReason, saved: bool) -> (ViewResult, ExitCode);
                   // wave-6 review (item 37):
                   pub fn for_folder(prepared: Option<&PreparedFolderCompare>, ignore_unimportant: bool,
                                     scanning: bool) -> ViewOutcome;
                   pub fn close_unopened(reason: CloseReason) -> (ViewResult, ExitCode); }
impl PreparedFolderCompare { pub fn view_verdict(&self, ignore_unimportant: bool, scanning: bool)
                                 -> CompareVerdict; }   // item 37
pub fn open_usage() -> String;   // usage of `hikasa gui open` / `hikasa-gui`, from the switch table
// OpenArgs also has `stdin_name: Option<String>` and `help: bool`.

// ipc
pub const MAX_FRAME_VALUES: usize = 1 << 18;          // JSON values per frame (counted before decoding)
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const MIN_PROTOCOL_VERSION: u32 = 1;
pub const MAX_PARKED: usize = 16;                     // client: messages for other requests kept while waiting
pub const MAX_SOCKET_PATH: usize = 100;
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);   // server: send timeout per connection
pub const WRITE_QUEUE: usize = 64;                    // server: frames queued per connection
pub const ABANDONED_EXIT_CODE: ExitCode = ExitCode::UnknownError;   // 100
impl Endpoint { pub fn verify_folder(&self) -> Result<(), IpcError>; }
impl Request { pub const fn type_name(&self) -> &'static str; }
impl Response { pub fn error(code: IpcErrorCode, message: impl Into<String>) -> Response; }
impl ViewKind { pub const ALL: [ViewKind; 13]; pub const fn as_str(self) -> &'static str;
                pub const fn is_served_in_m1(self) -> bool; }
impl CloseReason { pub const fn as_str(self) -> &'static str; pub fn parse(s: &str) -> Option<CloseReason>; }
impl ViewResult { pub fn folder_verdict_kind(&self) -> Option<CompareVerdict>;
                  pub fn close_reason(&self) -> Option<CloseReason>; }
impl IpcErrorCode { pub const ALL: [IpcErrorCode; 7]; pub fn parse(s: &str) -> Option<IpcErrorCode>; }
impl IpcClient { pub fn request(&mut self, request: Request) -> Result<(u64, Response), IpcError>;  // Error → Remote
                 #[doc(hidden)] pub fn connect_with_versions(/* … */); }
impl IpcServer { pub fn endpoint(&self) -> &Endpoint; }
impl ServerHandle { pub fn endpoint(&self) -> &Endpoint; }
impl ViewWaiter { pub fn request_id(&self) -> u64; }
pub fn peer_uid_matches(peer: Option<u32>, owner: u32) -> bool;
impl InstanceLock { pub fn file(&self) -> &File; }
// options.rs
pub const OPTIONS_FILE: &str = "options.toml"; pub const OPTIONS_SCHEMA_VERSION: u32 = 1;
pub fn parse_options(text: &str, origin: &str) -> Result<OptionsLoad, SessionError>;
impl ProgramOptions { pub fn stream_options(&self, threads: usize) -> StreamOptions;
                      pub fn content_budget_bytes(&self) -> u64; }
// formats.rs
pub const ORDER_FILE: &str = "order.toml"; impl FormatStore { pub fn dir(&self) -> &Path; }
// host.rs
impl Platform { pub const HOST: Platform; pub const fn path_style(self) -> PathStyle; pub const fn is_unix(self) -> bool; }
impl EnvFacts { pub fn new(platform: Platform, temp_dir: PathBuf) -> EnvFacts; }
```

**Deviations, interpretations and amendments:**

1. **`vfs_stat`** returns `dto::VfsStatDto` (`{spec, entry, capabilities}`, the §4.3.4 shape), not `VfsEntryDto`:
   the contract's signature could not produce `hikasa.vfs_stat.v1` without a second VFS call.
2. **`Endpoint::own_dir`** (Hikasa's own fallback socket folder, created and verified by the server) and
   **`Endpoint::uid`** (decision 1) are fields of `Endpoint`; `for_config` fills both (`uid` from `EnvFacts::uid`
   on Unix, also for `HIKASA_IPC_ENDPOINT`).
3. **§2.23 row 6** could never occur as written (`hk-<h16>.sock` is longer than `hikasa[-<h8>].sock`, so the first
   fallback is always longer than 100 bytes too). The row is corrected: a Unix path longer than 100 bytes becomes
   `/tmp/hikasa-<id>/hk-<h16>.sock` (Hikasa's own folder). The algorithm of §2.23 is unchanged.
4. **`SessionError::Schema`** (a newer `options.toml` or `order.toml`) and **`SessionError::EmptyConfigDir`** are
   additive variants. A **user format file** of a newer schema version is skipped with `user_format_skipped`
   (§2.12.4 step 2: "a file that fails to parse is skipped"; the built-ins stay usable), whereas a newer
   `options.toml` or `order.toml` stops the program (D-31). Both are tested.
5. **`OpenArgs::stdin_name`** (the `--stdin-name` of `gui open`, for `StdinSource`) and **`OpenArgs::help`**
   (`--help`, `-h`, `/?`; [`open_usage`] prints the usage) are additive fields.
6. **`ReportSettings`** carries the report title (routed through `ReportMeta::with_title`), the HTML style and
   `list_summary`. `list_summary` defaults to `true` and **no CLI flag changes it**: `--quiet` keeps the
   folder list's `#` summary line (§4.3.3, CLI invariant 3); `false` is an API-only option for embedders.
   `render_text_with` and `render_folder_with` replace the options' ignore-unimportant with the response's
   effective flag (item 20).
7. **Standard input** (decision 5 and consumer findings). `ServiceInit::stdin` is a lazy `StdinSource`: the
   service reads it at most once, the first time `parse_spec` sees `-` (also a `-` rules file), stamps it with the
   injected clock and serves it from then on (`Service::vfs()` includes it). Adapters pass a source
   unconditionally and never decide which arguments are specs; `format detect -` uses the name hint only and never
   reads standard input; `write_output("-")` never reads it either (the VFS refuses the target). No source, or a
   failing read, is an error of the command (exit 100). The **spool** of `gui open -` is
   `<cache>/stdin/<n>/<name>`: on Unix the cache folder must belong to the user and not be writable by others
   (the temp fallback `<temp>/hikasa-cache-<id>` must also be a real folder, not a symbolic link), `stdin/` is a
   real 0700 folder of the user, `<n>` is created fresh with mode 0700 (create-new: `mkdir` never follows a link;
   the first free number wins), and the file is created with `create_new` and mode 0600 (never through a link);
   any failure removes the file and the numbered folder. Planted links and foreign folders are refused, never
   used.
8. **Startup warnings** have one channel: `Service::startup_warnings()` is data, which adapters put at the start
   of every JSON envelope's `warnings` and print once to stderr in human mode; `Service::new` logs them at debug
   level only (no WARN line), and ignored option keys at debug level (`--verbose` shows them; adapters log nothing
   themselves).
9. **`format_mismatch` and the report header.** The session's own warnings (`format_mismatch`, rules-file keys)
   reach the JSON envelope and stderr through `TextCompareResponse::warnings`; they are not added to the RPT-004
   header, which lists the comparison's and the rendering's warnings (§1.7.3, session-review additions). The grammar-dependent report
   paths of §1.7.3 "Goldens" are covered by Service tests (tab size 2, a `lines_independent` normal diff, the
   `Rules: format: C, C++, C#, ObjC Source; …` line, the title in text and folder reports).
10. **Jobs.** `Service::spawn` runs the body on its own `hikasa-job-<id>` thread, outside the pool; the engines it
    calls enter the pool for their parallel sections (they take the pool explicitly, T7). A long job therefore no
    longer pins a worker for its whole duration (§5.1 amended). Residual limit: hikasa-text's parallel sections
    still occupy the workers while they run, so with a pool smaller than the number of concurrent long comparisons
    a small new view waits for the next section boundary (measured: 1.1 s instead of 2.0 s with 2 workers and two
    400k-line views; immediate with the default pool size). A separate `JobKind` for view support work is not
    added: `JobKind` is hikasa-core's enum (wave 1); such jobs use the kind of the view they serve.
    `JobHandle::wait` after `try_result` returned the result is `InvalidArgument`; `is_finished` is additive.
11. **IPC security on Unix** (decision 1). Before connecting, the client verifies the socket folder
    (`Endpoint::verify_folder`, `std` metadata only): it must be a folder owned by `Endpoint::uid` that is not
    group- or world-writable (`mode & 0o022 == 0`); Hikasa's own folder must also be a real folder with
    `mode & 0o077 == 0`; a symbolic link to the folder is accepted only when the link belongs to the same user;
    an unknown uid fails closed; a missing folder is `NotRunning`. Anything else is
    `IpcError::InsecureEndpoint` (exit 100 for every `gui` command) and nothing is sent, launched or awaited. The
    server creates its own folder with mode 0700 and create-new semantics, refuses a folder it does not own (or a
    symbolic link, or one others can enter), applies the same folder check before binding, removes only a socket
    at the socket path (any other file is an error and is never deleted), sets the socket to 0600 and checks that
    it belongs to `Endpoint::uid`. Per connection, credentials that cannot be read close the connection (fail
    closed); a platform whose credentials carry no uid is accepted and logged (documented limitation; the private
    folder still protects the endpoint). `XDG_RUNTIME_DIR` stays the preferred location; `HIKASA_IPC_ENDPOINT`
    endpoints are verified too (tests put them in private temporary folders).
12. **Protocol rule 2 is enforced by the library** (decision 2). A `ViewWaiter` dropped without `view_closed`
    sends `ViewClosed` with reason `abandoned`, no verdict and exit code 100 (`ABANDONED_EXIT_CODE`), also when a
    handler panic dropped it; when it is dropped before the immediate response was written it is held back and
    sent right after `ViewOpened`; when the `OpenView` failed nothing is sent. `ServerHandle::shutdown` (and
    dropping the handle) stops accepting, sends every waiter still outstanding `ViewClosed` (reason `shutdown`,
    exit 100: a GUI notifies its waiters with the views' real codes first, through `ViewOutcome::close`), closes
    every connection after its last frame and waits up to 2 s in total for the writers. Exactly one `ViewClosed`
    per wait: the first close wins, later `view_closed` calls are no-ops. So `gui open --wait` can never hang on a
    live GUI.
13. **Writes never block the GUI** (decision 2). Every frame of a connection goes through a bounded queue
    (`WRITE_QUEUE` = 64) to the connection's writer thread (`hikasa-ipc-write-<n>`), which writes with a send
    timeout (`WRITE_TIMEOUT` = 5 s); `view_closed` only queues. A full queue or a failed write drops the client
    (the connection is shut down). A protocol error closes the connection after `Error{protocol}` (id 0), also when
    a waiter still holds it. On Windows a named pipe cannot be shut down from another thread without `unsafe`:
    there a closed connection ends when the client disconnects (waiters are still notified).
14. **Client timeouts and queues** (decision 2). The client verifies the folder, connects, and decodes frames on a
    reader thread (`hikasa-ipc-read`) into a bounded channel; each call waits with one deadline
    (`crossbeam_channel::after(timeout)`), so the 10 s request timeout is the total of the call, whatever frames
    for other requests arrive; no clock is read in the crate (T1). At most `MAX_PARKED` (16) messages for other
    requests are kept, more is a protocol error. A server message with id 0 (§4.4.1: protocol errors) fails the
    pending call or wait as `IpcError::Remote` (exit 100, not 102). Any message other than `ViewClosed` or `Error`
    for an awaited request is a protocol error. Dropping the client shuts the connection down.
15. **Peak memory per frame** (decision 2). Before the two-phase decode, a frame's JSON values are counted (an
    O(n), allocation-free upper bound over the structural characters outside strings); more than
    `MAX_FRAME_VALUES` (262,144) is `Error{protocol}` and a close. One 16 MiB frame of tiny values therefore costs
    the 16 MiB read buffer, not a 0.5 GiB value tree; real messages hold a few hundred values.
16. **Pool panics** (decision 3). The service pool has a rayon `panic_handler`: a panic in a task given to
    `pool().spawn` is logged (`hikasa`, error level) and ends that task only; the worker and every other task keep
    running (rayon's default would abort the process). Panics inside `install`/`join`/`scope` still propagate to
    their caller, where every service method's and every `Service::spawn` job's `run_guarded` turns them into
    `ApiError::Internal` (§5.6 amended).
17. **Effective ignore-unimportant in folder results** (decision 4). `FolderCompareResponse::ignore_unimportant` is
    the request's flag or the `--criteria` word `ignore-unimportant`; `FolderCompareResponse::json` and
    `render_folder_with` use it in place of the caller's `DisplayOptions::ignore_unimportant`, so `result.verdict`,
    the statistics, the categories, the markers and the exit code always agree, also with the one-call
    `folder_compare`. Quick and text responses carry their flag the same way (`QuickDto::new(&resp)`).
18. **Launching** (decision 6). `Service::gui_launch()` gives the launcher `--config-dir <absolute dir>` for an
    explicit or portable configuration (the launched GUI then computes the same endpoint, D-47; a portable GUI
    found elsewhere on `PATH` would not see the marker) and `HIKASA_IPC_ENDPOINT` in the child's environment when
    the endpoint is overridden; the per-user default needs neither (the child inherits the environment).
    `find_gui_exe(facts)` returns `$HIKASA_GUI_EXE` alone when it is set (missing file → `None` → 103), else the
    first existing `gui_exe_candidates` entry; this makes the §6.3 launch and 103 tests independent of a real GUI
    built next to the CLI.
19. **Unix criteria flags** (decision 7). `FolderRulesPatch` has `unix_permissions`, `unix_owner` and
    `unix_group` (§1.6.8 item 24), applied after `unix`: `--permissions`, `--owner` and `--group` set one field
    each, so a rules file's or `--criteria`'s other unix tests stay; `--criteria` keeps setting `unix` (unlisted →
    off).
20. **`--gui` ↔ `OpenView`** (decision 7, D-50). `GuiOpenRequest::for_text` and `for_folder` turn a compare
    request plus its view words (`ViewWords`) into the open request: specs, kind, forced format, per-side
    encodings, rules files (raw, made absolute by `resolve_open`), rules patches, `--criteria`, ignore-unimportant,
    `--display`, `--context`, `--folders`, `--suppress-filters`, `--expand-all`. `--filters` of `folder compare`
    travels as `folder_rules.name_filter`; `OpenOptions::filters` is the legacy `gui open --filters` switch and
    wins when both are given. `Service::open_view_request(kind, specs, options)` is the inverse the GUI uses: it
    requires exactly two canonical absolute specs (relative specs, `-` and `clipboard://` are `InvalidArgument`,
    never resolved against the GUI's cwd, §4.4.3 rule 4) and returns the `TextCompareRequest` (with
    `ViewOptions` from `display`/`context_lines`, default all) or the `FolderCompareRequest` (with `DisplayOptions`
    from `display`/`folders`/`suppress_filters`, default differences with the folder structure, and
    `expand_all`). Words travel unparsed and are parsed by the same core parsers on both sides (T11).
21. **Local validation of `--gui`** (consumer finding). `gui_open` resolves the request (`resolve_open`) and then
    validates it (`check_open`: the `OpenView` mapping and its display words, the forced format, the encoding
    labels, the text rules, and for folders the rules file, `--criteria`, the patch and the text rules of
    `--content rules`) before it contacts or launches the GUI, so a typo exits 100 locally, with or without
    `--wait`. Rules files given as `-`, `clipboard://` or a path that is not valid UTF-8 are `InvalidArgument`.
22. **D-23 close rules** (decision 7). `ViewOutcome::close(reason, saved)` is the one implementation the GUI
    calls: `NoVerdict` (a file view not finished or cancelled) → 0; `LoadFailed` → 100 with reason
    `load_failed`; `Text{result, iu}` → `ExitCode::for_content`; `Folder{verdict}` → `ExitCode::for_verdict`. A
    folder view closed while it is still scanning reports `Folder { verdict: Incomplete }` → **100** (decision:
    an incomplete comparison cannot claim sameness, D-02).
23. **FC-062 with non-UTF-8 names.** `TextCompareRequest::for_specs(left, right)` (and `spec_os_string`) carry
    the exact OS path of local specs from `TreeRead::spec(id, side)`; `canonical()` is lossy for such names and
    would name another file. §1.10 is amended accordingly. No hikasa-folder change was needed.
24. **Second-instance forwarding.** `Service::forward_open(&open.request)` is what a second `hikasa-gui <args>`
    calls on `AlreadyRunning` (without `--solo`): the specs are resolved against its own cwd (absolute, §4.4.3
    rule 4), `-` is spooled, rules files are made absolute, nothing is launched or awaited; exit 0 on success,
    else `err.exit_code(ApiOp::GuiOpen { wait: false })`. The first instance resolves its own command line with
    `resolve_open` and maps it with `open_view_request`. §1.10 and §3.6 are amended.
25. **Vocabulary and defaults** (decision 7, T11). `parse_content_method` (`--content`), `parse_algorithm`
    (`--algorithm`, with the valid words), `parse_line_range` (`--lines a-b` with `1 ≤ a ≤ b`, or one line `n`;
    `5-`, `-3`, `0-1`, `9-2` are errors), `resolve_text_display` (a `--display` word, else `--context n` alone means
    `context` with n lines, else `mismatches` for JSON and `all` otherwise, §4.3.2) and `resolve_folder_display`
    (defaults `differences` and `compare_structure`, the §4.3.3 examples) are the core functions the CLI and the
    GUI use; `format_tokens` with an empty range (`1..=0`, `0..=0`, a range past the end) selects no line.
26. **Human output and escaping** (decision 7). hikasa-report exports `escape_controls`, `escape_terminal` and
    `escape_name` (§1.7.3, session-review additions); `api::human` formats the list commands with them (`format tokens`, `vfs ls`,
    `vfs stat`, `gui list`, `format list`, `format detect`), so file text and names never reach a terminal with raw
    control or bidirectional characters (§1.7.3 item 5).
27. **JSON helpers.** `JsonEnvelope::success_raw` (the raw view code of `gui open --wait`, which a newer GUI may
    send outside `ExitCode`), `JsonEnvelope::usage_error` (`hikasa.usage_error.v1`: a command line that does not
    parse, with `--json` anywhere in it; exit 100, D-35), `hikasa.completions.v1` (`{shell, script}`: CLI-000,
    every subcommand supports `--json`), `GuiOpenOutcome::exit_code`/`GuiOpenReport::exit_code`,
    `Service::gui_ping_dto` (with the endpoint text) and `Service::vfs_cat_dto`.
28. **Rules-file errors name the file.** A `--rules-file`/`--text-rules-file` that does not parse or validate is
    `ApiError::RulesFile { spec, source }`: the message starts with the canonical spec, JSON `error.spec` is set,
    kind and exit code are the source's.
29. **`options.toml` edge cases** (D-31, §5.5 rule 3). A known section with a non-table value
    (`text_editing = 5`) is an error; option keys match the table structure, so a quoted dotted key
    (`"text_editing.number_of_context_lines" = 9`) is one unknown key of an unknown section (warning, not applied);
    a negative `schema_version` is an error ("must be a non-negative integer"), not "newer".
30. **`gui open` parsing.** Inline values are split on the OS string, so `--config-dir=<non-UTF-8 path>` and
    `--log=<…>` work like their separate spellings (a non-UTF-8 value of a text switch stays `InvalidArgument`).
    On Windows an unknown `/name[=v]` whose name looks like a switch (ASCII letters, digits, `-`, `?`) is
    `InvalidArgument`; `/dir/file` and `/a.txt` stay root-relative paths. `--config-dir ""` is
    `SessionError::EmptyConfigDir`, never the current directory.
31. **Text rules of a folder comparison without `--content rules`** are reported with `switch_ignored` (they used
    to be ignored silently; a missing `--text-rules-file` is then not read at all).
32. **`write_atomic`** keeps at most `TEMP_NAME_PREFIX_MAX` (200) bytes of the target name in its temporary name,
    so every name the file system accepts can be written atomically.
33. **`NO_COLOR`** is an `EnvFacts` fact (`no_color`), so the CLI's colour decision is testable like every other
    ambient fact; the TTY test stays in the CLI (`std::io::IsTerminal`).
34. **`gui open` and the global flags** (§1.9.2 amendment). clap consumes the global flags that come directly
    after `open`, before the first positional argument; `parse_open_args` sees everything else. The CLI merges
    `OpenArgs::{json, quiet, no_color}` with the globals, and honours `OpenArgs::{config_dir, log, verbose}` too,
    because `parse_open_args` runs before `Service::new` (§1.9.1): the same flag given both ways with different
    values is a usage error (100). So the position of a flag never changes what it means.
35. **Rejected:** a `HIKASA_FIXED_NOW` clock override for reproducible CLI goldens (§6.1: CLI goldens drop the
    `Generated:` line; Appendix C item 8 keeps reproducible reports out of M1); splitting `folder_prepare` into a
    pure step and a loading step (it reads the rules files through the VFS and compiles the text rules; it is fast,
    and a GUI that may read a slow share calls it inside `Service::spawn` and sends the model to the UI thread —
    documented in §1.10 instead).
36. **Architecture test** (decision 8). The hardening of §6.1 (amended there) closes the bypasses the reviewers
    demonstrated; the checks remain a heuristic, with the limits listed in §6.1.
37. **Folder-view outcome and unopened views** (wave-6 GUI review, maintainer decisions 3 and 6; additive).
    The rule "a folder view that is still preparing, still scanning or was stopped reports `Incomplete` (100)"
    is core code, used by the GUI for both the status bar and the close code:
    `PreparedFolderCompare::view_verdict(ignore_unimportant, scanning) -> CompareVerdict` (`Incomplete` while
    `scanning` or when the model is not complete, else the model's verdict) and
    `ViewOutcome::for_folder(prepared: Option<&PreparedFolderCompare>, ignore_unimportant, scanning)` (`None`:
    still preparing or stopped while preparing → `Incomplete`). A view the GUI accepted over IPC but never
    opened (the `OpenView` reached the UI after quitting started, or the view was closed before it opened)
    closes with `ViewOutcome::close_unopened(reason) -> (ViewResult, ExitCode)`: no verdict, the given reason,
    exit 100, as the server reports the waiters it notifies at shutdown (§3.6). Architecture check 7 no longer
    takes the `'static` lifetime of a `&'static str` return type for a `static` item (self-test
    `check_7_does_not_take_the_static_lifetime_for_an_item`).

### 1.9 hikasa-cli (binary `hikasa`)

**Purpose.** A thin adapter (ARCH-L2, ARCH-012). Each subcommand parses arguments, builds one request, calls
one `Service` method, and renders the result through `hikasa-report` or the DTO serialiser. It returns the
`ExitCode` that the response (or `ApiError::exit_code`) supplies. The CLI contains no comparison logic and no
vocabulary tables (T11): `--display`, `--criteria`, `--unimportant`, size/date filter expressions and the
`gui open` arguments are parsed by core functions. The only conditionals it may have are about presentation:
TTY detection, colour, and choosing JSON or human output.
**Uses.** All core crates through `hikasa-session` (and direct type imports), clap, clap_complete, anyhow,
serde, serde_json, tracing, tracing-subscriber, jiff.

#### 1.9.1 Module tree

```
cli/hikasa-cli/src/
├── main.rs        capture_env → Cli::try_parse (usage error → exit 100, D-35) → tracing (--log, --verbose) →
│                  for `gui open`: normalize_legacy_args + parse_open_args (core) →
│                  StdinSource (lazy, --stdin-name hint; the service reads it on the first "-") → Service::new →
│                  dispatch inside run_guarded (panic → 100) → exit(code)
├── args.rs        clap derive: Cli, GlobalArgs, Command and every *Args struct
├── output.rs      Output: stdout/stderr writers, colour decision (TTY && !--no-color && NO_COLOR unset),
│                  JSON envelope, BrokenPipe policy (stop writing, keep exit code)
├── progress.rs    StderrProgress (ProgressSink): one updating line when stderr is a TTY; nothing otherwise
├── launcher.rs    ProcessGuiLauncher (GuiLauncher: find_gui_exe + std::process::Command detached with null
│                  stdio, with the GuiLaunch arguments and environment; sleep = std::thread::sleep)
└── cmd/
    ├── quick.rs   text.rs   folder.rs   format.rs   vfs.rs   gui.rs   completions.rs   version.rs
```

#### 1.9.2 Argument surface (clap derive)

```rust
/// `hikasa`: compare folders and files from the command line.
#[derive(Debug, clap::Parser)]
#[command(name = "hikasa", version, disable_version_flag = false, propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// Global flags (CLI-000, CLI-003 `--log`/`--verbose`).
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Machine-readable JSON on stdout (§4).
    #[arg(long, global = true)]
    pub json: bool,
    /// No progress, no non-error stderr output.
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,
    /// Write diagnostic logs to FILE.
    #[arg(long, global = true, value_name = "FILE")]
    pub log: Option<PathBuf>,
    /// Debug-level logs; also logs ignored option keys (D-31).
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Use DIR as the settings folder (portable behaviour).
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// Disable ANSI colours.
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Quick compare two files; exit code only (1/2/11/12/13/100).
    Quick(QuickArgs),
    /// Text tools.
    Text { #[command(subcommand)] cmd: TextCmd },        // TextCmd::Compare(TextCompareArgs)
    /// Folder tools.
    Folder { #[command(subcommand)] cmd: FolderCmd },    // FolderCmd::Compare(FolderCompareArgs)
    /// File formats.
    Format { #[command(subcommand)] cmd: FormatCmd },    // List | Show{name} | Detect(DetectArgs) | Tokens(TokensArgs)
    /// Generic VFS access.
    Vfs { #[command(subcommand)] cmd: VfsCmd },          // Ls{spec, --follow-symlinks} | Stat{spec, --follow-symlinks} | Cat{spec}
    /// Talk to the GUI over IPC.
    Gui { #[command(subcommand)] cmd: GuiCmd },          // Open(GuiOpenArgs) | Ping | List | Close{id} | Quit
    /// Print shell completions.
    Completions { shell: clap_complete::Shell },
    /// Print version information.
    Version,
}

/// `gui open`: every argument after `open` is passed raw to `hikasa_session::api::gui::normalize_legacy_args`
/// and `parse_open_args` (D-51), so that `hikasa gui open` and `hikasa-gui` accept the same forms.
/// (Amended, §1.9.3 item 9a: a `--` that clap consumes before the first value is restored, and `open` has no
/// clap `--help`/`--version` flags: `GuiCmd::Open` is `#[command(disable_help_flag = true,
/// disable_version_flag = true)]`, the shared parser's `--help` prints the usage.)
#[derive(Debug, clap::Args)]
pub struct GuiOpenArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
    pub args: Vec<OsString>,
}
```

| Subcommand | Arguments and flags (M1) | Service call → output |
|---|---|---|
| `quick <left> <right>` | `--size` \| `--crc` \| `--binary` \| `--rules` (default rules); `--ignore-unimportant` (alias `--iu`); `--format <id\|name>`; `--rules-file <toml>`; importance: `--unimportant <list>`, `--unimportant-text <text>`…, `--unimportant-regex <re>`…, `--compare-line-endings`; `--stdin-name <name>` | `Service::quick` → no stdout in human mode; `hikasa.quick.v1` with `--json` |
| `text compare <left> <right>` | `--format <id\|name>`; `--encoding <label>`, `--left-encoding <label>`, `--right-encoding <label>`; `--rules-file <toml>` (alias `--rules`); alignment: `--algorithm auto\|unaligned\|standard\|myers\|patience\|histogram`, `--skew <n>`, `--closeness`/`--no-closeness`, `--closeness-threshold <permille>`, `--never-align`; importance: `--unimportant <list>`, `--unimportant-text <text>`…, `--unimportant-regex <re>`…, `--compare-line-endings`, `--orphans-unimportant`; `--ignore-unimportant` (alias `--iu`); view: `--context <n>` (default from options, 3), `--display all\|mismatches\|context\|matches`; report: `--report unified\|side-by-side\|summary\|patch\|statistics` (default unified = annotated), `--patch-style normal\|context\|unified` (default unified), `--html`, `--html-mono`, `--line-numbers`/`--no-line-numbers`, `--width <n>`, `--wrap none\|character\|word`, `--title <text>`, `--output <file>`; `--stdin-name <name>`; GUI: `--gui`, `--wait`, `--no-launch` | `Service::text_compare` then `render_text` (human) or `text_compare_json` (`--json`); `--output` writes the report atomically and prints nothing; `--gui` → `Service::gui_open` (D-50) |
| `folder compare <left> <right>` | `--filters <masks>`; `--rules-file <toml>` (alias `--rules`); `--criteria <list>` (D-57); criteria: `--size`/`--no-size`, `--timestamps`/`--no-timestamps`, `--tolerance <secs>`, `--ignore-dst`, `--ignore-timezone`, `--filename-case`, `--attributes <letters>`, `--permissions`, `--owner`, `--group`; content: `--content none\|crc\|binary\|rules`, `--skip-if-quick-same`, `--no-override-quick`; text rules for `--content rules`: `--text-rules-file <toml>`, `--unimportant <list>`, `--unimportant-text <text>`…, `--unimportant-regex <re>`…; alignment: `--case-sensitive`\|`--case-insensitive`, `--align-extensions`, `--align-normalization`; filters: `--cutoff <expr>`, `--size-filter <expr>`, `--include-protected`; handling: `--follow-symlinks`, `--flatten`, `--no-scan-orphans`; view: `--display <filter>` (FC-055 names), `--folders always\|structure\|files-only`, `--ignore-unimportant`, `--suppress-filters`, `--expand-all` (accepted; human output is always fully expanded); report: `--report list\|side-by-side\|summary\|json` (default list), `--columns size,modified,…`, `--html`, `--html-mono`, `--title <text>`, `--output <file>`; GUI: `--gui`, `--wait`, `--no-launch` | `Service::folder_compare` then `render_folder` or `folder_compare_json`; `--gui` → `Service::gui_open` (D-50) |
| `format list` | — | `format_list` |
| `format show <id\|name>` | — | `format_show` (human: canonical TOML) |
| `format detect <file>` | `--stdin-name <name>` | `format_detect` |
| `format tokens <file>` | `--format <id\|name>`; `--encoding <label>`; `--lines <a>-<b>`; `--stdin-name <name>` | `format_tokens` (human: `L<line>:<start>-<end>\t<element>\t<escaped text>`) |
| `vfs ls <spec>` | `--follow-symlinks` | `vfs_ls` (human: `<kind><attrs> <size> <mtime> <name>`; sorted by name) |
| `vfs stat <spec>` | `--follow-symlinks` | `vfs_stat` |
| `vfs cat <spec>` | — | `vfs_cat` (raw bytes; `--json` → base64, D-32) |
| `gui open <args…>` | specs plus `--wait`; `--no-launch`; `--fv <type>` (`--fileviewer`); `--title1..4 <text>` (`--lefttitle`, `--righttitle`, `--centertitle`, `--outputtitle`); `--ro`, `--ro1`, `--ro2` (and their aliases); `--filters <masks>`; `--expandall`; `--format <id\|name>`; `--iu` (`--ignoreunimportant`, `--ignore-unimportant`); `--stdin-name <name>`; legacy spellings `-name`, `/name` (Windows); other CLI-003 switches are forwarded with warning `switch_ignored` | `parse_open_args` → `gui_open` → exit code of the view when `--wait` (D-23), else 0 |
| `gui ping` / `gui list` / `gui close <id>` / `gui quit` | — | `gui_ping` / `gui_list` / `gui_close` / `gui_quit`; never launch; no GUI → 100 (D-22) |
| `completions <shell>` | bash, elvish, fish, powershell, zsh | `clap_complete::generate` to stdout |
| `version` | — | `hikasa 0.1.0` / `hikasa.version.v1` |

Flag-to-request mapping (no logic beyond this table): `--unimportant` → `UnimportantItem::parse_list`
(core) into `TextRulesPatch::unimportant`; `--unimportant-text t` → `TextPattern { pattern: t, regex: false,
match_case: true }`; `--unimportant-regex r` → `TextPattern { pattern: r, regex: true, match_case: true }`;
`--orphans-unimportant` → `orphans_always_important = Some(false)`; `--encoding e` → both entries of
`TextCompareRequest::encodings`, then `--left-encoding`/`--right-encoding` replace one; `--criteria` →
`FolderCompareRequest::criteria` (parsed by the service with `Criteria::parse_list`); `--display` →
`FolderDisplayFilter::parse` or the text mode table of §2.18; `--cutoff` → `DateFilter::parse` with the clock's
zone; `--size-filter` → `SizeFilter::parse`; individual criteria flags → `FolderRulesPatch` fields.

*Amended in the wave-4 session review (§1.8.7):* `--permissions`, `--owner`, `--group` → `unix_permissions`,
`unix_owner`, `unix_group` (one field each); `--content` → `api::parse_content_method`; `--algorithm` →
`api::parse_algorithm`; `--lines` → `api::parse_line_range`; `--folders` → `api::parse_folder_mode`; the text
display (`--display`, `--context`, JSON or human) → `api::resolve_text_display`; the folder display (`--display`,
`--folders`, `--suppress-filters`; default `differences` with the folder structure) →
`api::resolve_folder_display`, whose ignore-unimportant the response replaces with the effective flag; `--gui` →
`GuiOpenRequest::for_text`/`for_folder` with the view words, then `gui_open_with` (local validation first);
standard input → one `StdinSource` passed unconditionally; the launcher → `find_gui_exe` and `Service::gui_launch`.
The human lines of `format tokens`, `vfs ls`, `vfs stat`, `gui list`, `format list` and `format detect` are
`api::human`; `format show` prints `format_toml`. With `--json` (before any `--`: after it `--json` is an
operand), a usage error prints `JsonEnvelope::usage_error` (`hikasa.usage_error.v1`, exit 100; the message is
clap's first paragraph in one line) and `completions` prints `hikasa.completions.v1` (`{shell, script}`). Every
envelope's `warnings` start with `Service::startup_warnings()`, which human mode prints once to stderr — except
`version` and `completions`, which run without the service (§1.9.3 item 1). For `gui open`, the arguments after
`open` reach `parse_open_args` exactly as written, a leading `--` included, except the global flags clap sees
directly after `open`; those and the `OpenArgs` flags after the first positional are merged (the same flag given
both ways with values that name different paths is a usage error, 100). A global value flag (`--config-dir`,
`--log`) repeated at different clap levels with different values is a usage error as well (clap alone would keep
the innermost one), so a flag means the same wherever it stands (§1.9.3 item 9d).

**CLI invariants** (checked by tests in §6.3):
1. The exit code always comes from `ExitCode` (§2.21). clap usage errors give 100, and `--help`/`--version` give 0
   (`--version` prints `hikasa <version>` before or after a subcommand; amended §1.9.3 item 9c).
2. With `--json`, stdout holds exactly one JSON document followed by a newline, even on error. Logs and
   progress go to stderr. *Amended (§1.9.3 item 9c):* `--help` (and `gui open --help`) prints its text also with
   `--json` — help is never JSON; `--version` with `--json` prints the `hikasa.version.v1` document.
3. `--quiet` never changes stdout or the exit code.
4. `--output FILE` writes atomically. On error, FILE is untouched. *Amended (§1.2.7 item 12):* through a symbolic
   link the file it points to is replaced and the link stays; a FIFO or a device is written to directly and never
   replaced (not atomic).
5. A broken pipe on stdout stops writing and does not change the exit code.
6. `--gui` never compares locally: it sends the request to the GUI (D-50) and behaves like `gui open`.

#### 1.9.3 Accepted additions and deviations (wave 5)

The review of wave 5 (hikasa-cli) accepted the following behaviour, additions and deviations. They are part of
the contract and win over the text of §1.9.1–§1.9.2 where they refine it.

1. **`version` and `completions` skip the service.** They read no configuration, so they work with a broken or
   newer `options.toml`; consequently their envelopes carry no startup warnings (an exemption from §1.9.2 and
   §1.8.7 item 8) and their errors use `ApiOp::Other`.
2. **`--output` with `--json`.** The report goes to the file and the envelope (the comparison's JSON result) to
   stdout; without `--json`, `--output` prints nothing.
3. **`--gui` conflicts.** The report flags (`text compare`: `--output`, `--report`, `--patch-style`, `--html`,
   `--html-mono`, `--line-numbers`, `--no-line-numbers`, `--width`, `--wrap`, `--title`; `folder compare`:
   `--output`, `--report`, `--columns`, `--html`, `--html-mono`, `--title`) conflict with `--gui`, and `--wait`
   and `--no-launch` require it: a clap usage error, exit 100.
4. **Golden normalisation beyond §6.1** (amended there): CLI goldens also drop HTML's `Generated` header row,
   mask the machine's uid, gid and status-change times in `vfs` documents (`<UID>`, `<GID>`, `<CTIME>`), collapse
   the column padding that follows a replaced `<TMP>` (its width depends on the folder's length), and drop the
   header line of a narrow side-by-side report, which cuts the folder at a length-dependent place.
5. **Panic → 100 is unit-tested** (`a_panic_in_a_command_exits_100`: `run_guarded` turns a panic into
   `ApiError::Internal`, whose code is 100 for every `ApiOp`), not end to end: the binary has no test-only switch
   that panics inside a command, and adding one would ship a fault switch.
6. **Human-output choices.** `quick`, `gui open`, `--gui`, `gui close` and `gui quit` print nothing on stdout;
   `gui ping` prints `<server>\t<protocol version>`; the other list commands print the `api::human` lines. Every
   failed command prints one `hikasa: error: …` line on stderr (also with `--quiet` and `--json`); warnings are
   `hikasa: warning: …` lines (not with `--quiet`; with `--json` they are in the envelope); progress is one
   updating stderr line on a terminal. The folder list keeps its `# …` summary line with `--quiet` (stdout never
   depends on `--quiet`).
7. **Performance gates through the binary.** `cli/hikasa-cli/tests/perf_gates.rs` (ignored; `cargo test
   --release -p hikasa-cli --test perf_gates -- --ignored`) generates the G1 input (hikasa-text's own generator)
   and the G2 trees (the shape of `folder_scan_100k`) under the temporary folder and checks the §6.4 thresholds
   through the `hikasa` binary: G1 < 3 s end to end for auto, standard and identity, with the peak-RSS bound on
   Linux; G2 < 1.5 s warm (the cold figure needs dropped caches, i.e. root). No CI job runs them; the measured
   figures are recorded in docs/12 §4 and the CHANGELOG.
8. **Test GUI.** The launch tests are the `harness = false` test target `cli_gui_launch`, whose binary is also
   the scripted GUI (`HIKASA_TEST_FAKE_GUI=1`, launched through `HIKASA_GUI_EXE=<its own path>`); no fake GUI is
   a product binary (§6.3 `cli_gui.rs`: "test support binary").
9. **Review fixes** (maintainer decisions of the wave-5 review):
   a. `gui open -- -x a.c` behaves like `hikasa-gui -- -x a.c` (D-51): the arguments after `open` reach the shared
      parser unchanged, a leading `--` included; inside them only the shared parser's switches exist, so
      `--version` is an unknown switch (100) wherever it stands after `open`.
   b. `--json` and `--no-color` are recognised only before the first `--`; when the `gui open` arguments do not
      parse, a legacy `-json` (`/json` on Windows) among them also selects `hikasa.usage_error.v1`; the usage
      error's message is clap's whole first paragraph in one line (e.g. `the following required arguments were
      not provided: <RIGHT>`).
   c. `--version` (`-V`) prints `hikasa <version>` at every level (clap printed `hikasa-quick 0.0.0`); with
      `--json` it prints the `hikasa version --json` document. `--help` is text also with `--json` (invariant 2
      amended).
   d. A global value flag given at several levels with values naming different paths is a usage error (100);
      values are compared as paths made absolute against the current folder (`cfg`, `./cfg/` and `<cwd>/cfg`
      agree), and existing paths also after resolving symbolic links. Inside the `gui open` arguments a repeated
      switch keeps the shared parser's rule (the last value wins, as for `hikasa-gui`).
   e. `quick --crc|--binary` on a file that cannot be read (result `error`, exit 100) prints the error on stderr
      (also with `--quiet`); the JSON document has `ok: false` and `error` (the result's `ErrorReport`), and keeps
      the result, whose `result.result` is `error` (§4.2, §4.3.1).
   f. A GUI executable that exists but cannot be started (no execute permission, a missing interpreter, any
      other spawn failure) is "no usable GUI executable": exit 103 with `ApiError::GuiNotStartable` (additive in
      hikasa-session; kind of the OS error; message `cannot start the GUI executable: <path>: <reason>`;
      `connect_or_launch` maps `LaunchError::Spawn` to it instead of `Ipc(Io)`/100). `find_gui_exe` makes a
      relative path absolute against `EnvFacts::cwd` (checked and started the same way, never searched on `PATH`)
      and skips sibling and `PATH` candidates without an execute bit (like `which`); an override that is a file
      is returned as is, so its spawn error names it.
   g. `--output` through a symbolic link or onto a FIFO or device: §1.2.7 item 12.
   h. One failed write to stdout gives one error line: the first failure is final for the stdout writer and the
      final flush reports only a failure no command has seen.
   i. A failure before the command runs (the `--log` file) is an `ApiError` (`Vfs` with the file's path, or
      `Internal` when the logger cannot be installed) reported through `cmd::fail` and the session's envelope
      helpers. Progress units are hikasa-core's own names (the serde names of `Unit`), not a table in the
      adapter.
10. **Deferred and rejected review items.**
    a. *Deferred:* checking a layout/format combination (`--report patch --html`, `folder compare --report list
       --html`) before the comparison runs needs an additive `TextLayout::supports(OutputFormat)` /
       `FolderLayout::supports` in hikasa-report; a table in the adapter would be a spec rule of its own (T11).
       Until then the renderer refuses the combination after the comparison: exit 100, `--output` untouched.
    b. *Rejected (documented instead):* removing the spooled standard input of an interrupted `gui open -
       --wait` (Ctrl-C, SIGTERM) needs a signal handler, which std does not offer, `unsafe` is not allowed
       outside `mmap.rs`, and no signal crate is approved; docs/12 §1.1 documents the leftover
       `<cache>/stdin/<n>/` folder. A later session change may sweep stale spool folders when spooling.

### 1.10 hikasa-gui: contracts it needs (implemented from wave 6)

| GUI need | Contract |
|---|---|
| One service shared with the CLI | `hikasa_session::api::Service` (created with `capture_env`, `SystemClock`, `StdProcessRunner`); every view action is a `Service` call, as ARCH-013 requires. |
| UI thread never blocks (00 §4) | `Service::spawn(kind, title, f)` → `JobHandle<T>`: `cancel()`, lossy `progress()` channel, `try_result()` polled per frame, panics → `ApiError::Internal`. |
| First screen of a 1M-line comparison < 1 s (ARCH-GUI-003) | `Service::text_compare(req, job, Some(observer))`: `CompareObserver::loaded` (documents for unaligned panes at once), `phase`, `partial` (confirmed run prefixes); then the final `Arc<TextComparison>`. |
| Virtualised text panes (ARCH-GUI-002/003) | `TextComparison::row_count`/`row` (O(1), < 200 ns), `document(side).line(i)` (rope), `spans(side, line)` after `prepare_spans` on a worker, `inline(row)` (lazy, p99 < 50 µs), `view(&ViewOptions)` → `FilteredView::get(pos)` (O(log k)), `next_section`, `row_for_line`, `stats`. `TextComparison` is `Send + Sync` and lives in an `Arc` shared by the UI and workers. |
| Thumbnail (TC-032) | `TextComparison::thumbnail(height_px, ignore_unimportant)` in O(sections + buckets). |
| Filtered gap bars (TC-035) | `ViewOptions::expanded` row ranges; `FilteredView::position_of` returns the gap that hides a row. |
| Folder tree with incremental updates | `Service::folder_prepare` → `PreparedFolderCompare::model()` (an `Arc<FolderModel>` to render at once); `Service::spawn` runs `folder_run` with `ChannelObserver::bounded(n)`. Per frame: if `model.generation()` changed, `model.read()` → `visible_rows(opts, expanded, &mut rows)` (O(visible)), `node(id)`, `colour(id, side, iu)`, `stats(iu)`; all O(1) per row on partial trees. Events are id-only hints. |
| Cancellation (Stop button) | `JobHandle::cancel`; the folder model stays consistent (`is_complete() == false`, unscanned folders `Unknown`). |
| Opening a file pair (FC-062) | `TreeRead::spec(id, side)` for both sides → `TextCompareRequest::for_specs(&left, &right)` (exact for non-UTF-8 local names, §1.8.7 item 23). |
| IPC server, single instance (ARCH-090) | `Service::endpoint()` → `IpcServer::bind` (instance lock per configuration, D-47) → `spawn(handler)` → `ServerHandle`; `ServerHandle::shutdown` on exit. The GUI implements `IpcHandler`, turns each `OpenView` into its view's request with `Service::open_view_request(kind, &specs, &options)` and keeps `ViewWaiter`s per view; on close it calls `let (result, code) = outcome.close(reason, saved)` (`ViewOutcome`, the D-23 rules) and `waiter.view_closed(view, result, code)`. A dropped waiter reports `abandoned` (100) and `shutdown` notifies the rest (§1.8.7 items 12, 22). |
| Second-launch forwarding and the legacy command line | `hikasa-gui <args>` → `normalize_legacy_args` + `parse_open_args` (the CLI's own parser, D-51) → `ServiceInit` with `OpenArgs::config_dir` and a lazy `StdinSource` → on `AlreadyRunning` (without `--solo`), `Service::forward_open(&open.request)` (absolute specs, spooled `-`, no wait), exit 0 or `err.exit_code(ApiOp::GuiOpen { wait: false })`; otherwise become the server and open the command line's view with `resolve_open` + `open_view_request`. A GUI launched by the CLI receives `--config-dir` for explicit and portable configurations (§1.8.7 items 18, 24). |
| Folder rules files on slow shares | `folder_prepare` reads the rules files through the VFS and compiles the text rules; it is fast, and a GUI that may read a slow share calls it inside `Service::spawn` and sends the model to the UI thread (§1.8.7 item 35). |

### 1.11 Stub crates

`hikasa-table`, `hikasa-hex`, `hikasa-picture`, `hikasa-metadata` and `hikasa-script` stay as they are:
crate docs and `#![forbid(unsafe_code)]`, with no dependencies. They are not dependencies of any M1 crate. The
architecture test asserts that they stay empty of `pub` items until their milestone, so that no unreviewed API
appears.

---

## 2. Normative algorithms

Each subsection is normative. Tests must be derived from the tables. "✓" means match / same, "✗" means no match / different.

### 2.1 Spec parsing and canonical form (ARCH-020, VFS-010, D-05, D-06)

**`Spec::parse(input, ctx)`** in order:

1. If `input` contains NUL → `Nul`. If `input.trim()` is empty → `Empty`. The input is **not** trimmed
   otherwise, because leading and trailing spaces are legal in file names.
2. `input == "-"` → `Stdin`.
3. ASCII-case-insensitive `input == "clipboard://"` → `Clipboard` (`clipboard://x` → `MalformedUrl`).
4. ASCII-case-insensitive prefix `profile:` → `Profile`. `name` is the text before the first `?`, and it must be
   non-empty (`MalformedProfile`). `path` is the text after `?`, or `None` when there is no `?`.
5. If `input` matches `^[A-Za-z][A-Za-z0-9+.-]*://`, then:
   - with a one-letter scheme and `PathStyle::Windows` → it is a drive letter; continue at step 6;
   - `http`/`https` → `Http`. The authority is parsed as below (user information included: `user` and
     `password` as for `Remote`); `rest` is everything from the first `/`, `?` or `#` after it;
   - a `RemoteScheme` → `Remote`. Authority = text up to the first `/`. `userinfo@hostport` is split at the
     **last** `@`. `user[:password]` is split at the first `:`. `host[:port]` can use `[v6]:port`; the port
     must be 1–65535 (`MalformedUrl`). The host must be non-empty. `path` = the rest, verbatim;
   - otherwise → `UnknownScheme`.
6. Local: `normalize_local(input, ctx.style, ctx.cwd, ctx.home)`.

**`normalize_local` — Unix style**

1. `~` or `~/…` → `home` + rest (`HomeUnknown` if `home` is `None`). `~name…` → `TildeUser`.
2. A relative path is prefixed with `cwd` + `/` (`RelativeWithoutCwd` if `cwd` is `None`).
3. Split on `/`. Drop empty components and `.`. Keep `..` (D-06), except that a `..` directly under the root
   is dropped (`/../x` → `/x`). Join with `/`. Strip a trailing `/` except for the root.

**`normalize_local` — Windows style**

1. Replace every `/` with `\`. Expand `~` as on Unix.
2. Classify:
   - `\\?\…` (verbatim) → return the input unchanged;
   - `\\.\…` → `DevicePath`;
   - `\\server\share\…` → UNC root `\\server\share` (`UncWithoutShare` when the share is missing);
   - `X:\…` → drive root, with the letter upper-cased;
   - `X:rest` → `cwd` + rest if `cwd` is on drive X (case-insensitive), else `DriveRelative`;
   - `\rest` → root of `cwd` + rest;
   - anything else is relative → `cwd` + `\` + input.
3. Drop empty and `.` components. Resolve `..` lexically (pop; at the root, ignore). Strip the trailing `\`
   except at `X:\`.

**Canonical form** (`Display`): `Local` → the stored path (lossy display for non-UTF-8 paths, with
`is_lossy() == true`); `Stdin` → `-`; `Clipboard` → `clipboard://`; `Profile` → `profile:<name>[?<path>]`;
`Remote` → `<scheme>://[<user>@]<host>[:<port>]<path>` (lower-case scheme and host, **never** a password);
`Http` → `http[s]://[<user>@]<host>[:<port>]<rest>` (never a password). A lossy `Local` spec (non-UTF-8 path)
cannot be serialised (`Serialize` returns an error), so it is never persisted.

INV-SPEC-4 (round trip): for every UTF-8 input that parses, `parse(canonical(s), ctx) == s.without_secrets()`
(`without_secrets` clears the password of both `RemoteUrl` and `HttpUrl`).

| # | Input | Style | cwd | home | Result |
|---|---|---|---|---|---|
| 1 | `/home/u/x` | Unix | – | – | `Local(/home/u/x)` |
| 2 | `/home/u//x/./y/` | Unix | – | – | `/home/u/x/y` |
| 3 | `/a/../b` | Unix | – | – | `/a/../b` (kept, D-06) |
| 4 | `/../x` | Unix | – | – | `/x` |
| 5 | `x/y` | Unix | `/w` | – | `/w/x/y` |
| 6 | `x/y` | Unix | none | – | `RelativeWithoutCwd` |
| 7 | `~` / `~/x` | Unix | – | `/home/u` | `/home/u` / `/home/u/x` |
| 8 | `~bob/x` | Unix | – | `/home/u` | `TildeUser` |
| 9 | `a\b` | Unix | `/w` | – | `/w/a\b` (backslash is a normal character) |
| 10 | `-` | any | – | – | `Stdin`, canonical `-` |
| 11 | `CLIPBOARD://` | any | – | – | `Clipboard`, canonical `clipboard://` |
| 12 | `C:\x\y` / `c:/x/y` | Windows | – | – | `C:\x\y` |
| 13 | `C:\a\..\b` | Windows | – | – | `C:\b` |
| 14 | `\\srv\share\x` / `//srv/share/x` | Windows | – | – | `\\srv\share\x` |
| 15 | `\\srv` | Windows | – | – | `UncWithoutShare` |
| 16 | `\\?\C:\x\..\y` | Windows | – | – | unchanged (verbatim) |
| 17 | `\\.\pipe\x` | Windows | – | – | `DevicePath` |
| 18 | `C:x` | Windows | `C:\w` | – | `C:\w\x` |
| 19 | `C:x` | Windows | `D:\w` | – | `DriveRelative` |
| 20 | `\x` | Windows | `D:\w` | – | `D:\x` |
| 21 | `C://x` | Windows | – | – | `C:\x` (one-letter scheme = drive) |
| 22 | `C://x` | Unix | `/w` | – | `UnknownScheme` |
| 23 | `ftp://user:pw@Host/p` | any | – | – | `Remote{ftp, user, password: Secret, host: "host", path: "/p"}`, canonical `ftp://user@host/p` |
| 24 | `sftp://u@h:2222/p` | any | – | – | `Remote{sftp, port: 2222}` |
| 25 | `sftp://u@h:99999/p` | any | – | – | `MalformedUrl` (port) |
| 26 | `s3://bucket/prefix/` | any | – | – | `Remote{s3, host: "bucket", path: "/prefix/"}` |
| 27 | `profile:My Prof?dir/f.txt` | any | – | – | `Profile{name: "My Prof", path: Some("dir/f.txt")}` |
| 28 | `https://Example.com/a?b` | any | – | – | `Http`, canonical `https://example.com/a?b` |
| 29 | `foo://x` | any | – | – | `UnknownScheme` |
| 30 | `` / `   ` | any | – | – | `Empty` |

### 2.2 File masks (FC-050, FC-051; D-07, D-08)

#### 2.2.1 Item syntax

```
list       := item (";" item)*                      empty items (after trimming) are ignored
item       := WS* ["-"] body WS*                   WS = ASCII space or tab; "-" = exclude
body       := [sep] components [sep]               a trailing sep marks a FOLDER mask; a leading sep anchors at the base
components := comp (sep comp)*
sep        := "\" | "/"
comp       := "..."                                 zero or more folder levels (anywhere)
            | "."                                   base folder (first component only)
            | pattern
pattern    := atom+
atom       := "?" | "*" | set | literal-char
set        := "[" ["!"] setbody "]"                 "]" immediately after "[" or "[!" is literal;
                                                     "a-z" is a range; "-" first or last is literal;
                                                     "\" and "/" inside a set are literal
```

Errors: `[` with no closing `]` → `UnclosedBracket`; a range with `lo > hi` → `InvalidRange`; an empty
component (`a\\b`, a lone `-`) → `EmptyComponent`; `.` anywhere but first, or `..` anywhere → `DotComponent`.
`[^a]` is a set containing `^` and `a`, because only `!` negates.

#### 2.2.2 Component matching

The name and every literal atom are first converted to NFC (`case::nfc`). For `Insensitive`, two chars are equal
iff `simple_lower(a) == simple_lower(b)`. A char `c` is in a set range `[lo-hi]` iff some
`x ∈ {c, simple_lower(c), simple_upper(c)}` satisfies `lo ≤ x ≤ hi`. (With `Sensitive`, only `x = c` is
used.) `?` matches exactly one `char` (Unicode scalar value). `*` matches any run of chars, including the
empty run.

Matching uses the linear greedy algorithm with a single backtrack point (O(|pattern|·|name|) worst case):

```
match(P, N):
  i = 0; j = 0; star = None; mark = 0
  while j < len(N):
    if i < len(P) and P[i] != '*' and atom_matches(P[i], N[j]): i += 1; j += 1
    elif i < len(P) and P[i] == '*': star = Some(i); mark = j; i += 1
    elif let Some(s) = star: i = s + 1; mark += 1; j = mark
    else: return false
  while i < len(P) and P[i] == '*': i += 1
  return i == len(P)
```

Extension rules apply to the component that is matched against the item's own name, and to every other
component in the same way:

- `has_extension(n)` ⇔ there is a `.` at char index `k` with `0 < k < len(n) − 1`. So `.bashrc` and `a.`
  have no extension.
- **Trailing-dot rule.** If the pattern text ends with a literal `.`, then `P` matches `n` iff
  `match(P without the final ".", n) && !has_extension(n)`.
- **Dot-star rule.** If the pattern text ends with `.*`, then `P` matches `n` iff
  `match(P, n) || (match(P without ".*", n) && !has_extension(n))`.

#### 2.2.3 Path matching

- A **simple** mask (one pattern component, no separator) matches the item's **name** at any depth.
- An **anchored** mask (`p\f`, `.\f` or `\f`) matches the item's `RelPath` components starting at the base.
  A leading `.` component is removed. `...` matches zero or more components, using the same greedy algorithm
  with components as atoms.
- A mask that starts with `...` is unanchored by construction.
- `target` decides applicability: file masks never match folders, and folder masks never match files.
- Implementation (normative for performance, not for results): `Mask::matches(parent, name)` walks
  `parent.components()` followed by `name` without building a joined path; per-char folding uses
  `case::simple_lower` on the fly; `case::nfc` borrows for ASCII names and for names that pass
  `unicode_normalization::is_nfc_quick`, so the common case allocates nothing. Simple masks compare only
  `name`. Bench `mask_match` (§6.4): ≥ 20 M matches/s on a 20-item list over ASCII names, where one match is
  one name checked against the **whole** 20-item list (`NameFilter::file`/`folder`, `MaskList::matches_name`).

#### 2.2.4 Examples (normative test table)

| # | Mask | Item (kind, RelPath) | Case | Result | Rule |
|---|---|---|---|---|---|
| 1 | `*.c` | F `main.c` | I | ✓ | simple |
| 2 | `*.c` | F `src/util/x.c` | I | ✓ | simple masks match at any depth |
| 3 | `*.c` | F `x.C` | I | ✓ | case-insensitive |
| 4 | `*.c` | F `x.C` | S | ✗ | case-sensitive |
| 5 | `*.c` | D `lib.c` | I | ✗ | file masks never match folders |
| 6 | `?.txt` | F `a.txt` / F `ab.txt` | I | ✓ / ✗ | `?` = one char |
| 7 | `*.` | F `Makefile` | I | ✓ | trailing dot = no extension |
| 8 | `*.` | F `a.out` | I | ✗ | |
| 9 | `*.` | F `.bashrc` | I | ✓ | a leading dot is not an extension |
| 10 | `*.*` | F `Makefile` | I | ✓ | dot-star rule |
| 11 | `*.*` | F `a.tar.gz` | I | ✓ | |
| 12 | `abc.*` | F `abc` / F `abcd` | I | ✓ / ✗ | |
| 13 | `a?c.*` | F `a.c` | I | ✗ | `a.c` has an extension, so the dot-star rule does not apply |
| 14 | `[ab]*` | F `apple` / F `cherry` | I | ✓ / ✗ | set |
| 15 | `[a-c]?` | F `b1` | I | ✓ | range |
| 16 | `[!a-c]*` | F `dog` / F `cat` | I | ✓ / ✗ | negated set |
| 17 | `[[]1].txt` | F `[1].txt` | I | ✓ | `[[]` = literal `[` |
| 18 | `[]a]x` | F `]x` | I | ✓ | `]` first in a set is literal |
| 19 | `[a-]` | F `-` | I | ✓ | trailing `-` literal |
| 20 | `[A-Z]*` | F `zeta` | I | ✓ | range tested against `simple_upper` too |
| 21 | `src\` | D `src` / D `lib/src` | I | ✓ / ✓ | simple folder mask at any depth |
| 22 | `src\*.c` | F `src/a.c` / F `lib/src/a.c` | I | ✓ / ✗ | relative masks are anchored (D-08) |
| 23 | `src/*.c` | F `src/a.c` | I | ✓ | `/` ≡ `\` |
| 24 | `.\*.c` | F `a.c` / F `src/a.c` | I | ✓ / ✗ | base folder only |
| 25 | `\docs\` | D `docs` / D `a/docs` | I | ✓ / ✗ | a leading separator anchors |
| 26 | `...\test\*.py` | F `test/a.py` / F `pkg/sub/test/a.py` / F `test/x/a.py` | I | ✓ / ✓ / ✗ | `...` = any depth |
| 27 | `src\...\*.rs` | F `src/a.rs` / F `src/x/y/a.rs` / F `a.rs` | I | ✓ / ✓ / ✗ | |
| 28 | `café.txt` (NFC) | F `cafe\u{301}.txt` (NFD) | S | ✓ | NFC normalisation |
| 29 | `STRASSE` | F `straße` | I | ✗ | 1:1 folding only, no expansion |
| 30 | `a*b*c` | F `aXbYc` / F `abc` / F `acb` | S | ✓ / ✓ / ✗ | |
| 31 | `*` | F `x` / D `x` | I | ✓ / ✗ | |
| 32 | `*\` | D `x` | I | ✓ | |
| 33 | `[z-a]` | — | — | `InvalidRange` | |
| 34 | `a[b` | — | — | `UnclosedBracket` | |
| 35 | `a\\b` | — | — | `EmptyComponent` | |
| 36 | `x\..\y` | — | — | `DotComponent` | |
| 37 | `  *.h ` | F `a.h` | I | ✓ | items are trimmed |

Toolbar parsing example: `*.c;*.h;-*.bak;-obj\;src\` gives include files `[*.c, *.h]`, exclude files `[*.bak]`,
exclude folders `[obj\]` and include folders `[src\]`. `to_filter_string()` emits the canonical order: include
files, include folders, exclude files, exclude folders, so it emits `*.c;*.h;src\;-*.bak;-obj\`
(INV-MASK-2: parse ∘ to_filter_string is the identity on lists).

### 2.3 NameFilter evaluation (FC-050, FC-051, FC-054; D-09, D-44, D-45)

`FolderVerdict::Base` denotes the base folder: its files are subject to file masks, but it does not propagate
subtree inclusion. Evaluation is top-down and **per side**: each side has its own verdict chain, because names
on the two sides can differ in case, normalisation or extension. `p/n` below is the item named `n` inside the
folder `p` (`p` = the parent's `RelPath`); no joined path is built (§2.2.3).

```
folder(p, n, parent) -> FolderVerdict
  1. parent == Excluded                                      → Excluded
  2. ∃ m ∈ exclude_folders: m.matches(p, n)                  → Excluded      (exclusion always wins)
  3. include_folders is empty                                → Included
  4. parent == Included                                      → Included      (subtree rule, D-09)
  5. ∃ m ∈ include_folders: m.matches(p, n)                  → Included
  6. ∃ m ∈ include_folders: m is unanchored, or m.may_match_below(p, n) → PassThrough
  7. otherwise                                               → Excluded

file(p, n, parent) -> bool
  parent ∈ {Excluded, PassThrough}                           → false
  ∃ m ∈ exclude_files: m.matches(p, n)                       → false
  include_files is empty or ∃ m ∈ include_files: m.matches(p, n)
```

How the scanner uses the verdicts (FC-054, D-44, D-45):

- A folder node is listed on side S iff S's verdict is not `Excluded`. A folder node is skipped entirely (its
  children are not listed) only when every present side is `Excluded`. *Amended (§1.6.8 item 10):* a folder whose
  present sides are all filtered — by any filter, e.g. SYSTEM + HIDDEN on every side — is not listed either; a
  side hidden by the other filters only is listed whenever the pair is not filtered on every side (FC-054), so a
  folder protected on one side only compares its contents instead of reporting them as orphans.
- **Prefilter (D-45).** When listing a folder of side S, the scanner passes `ListOptions::prefilter` =
  `|name, kind| !(excluded by name masks)`, i.e. `kind == Dir ? folder(p, name, v) != Excluded : file(p, name, v)`
  with `v` = the folder's verdict on S. Declined entries arrive in `Listing::name_only` and become name-only
  entries (`stat_skipped = true`). Symlinks and entries of unknown kind are always stat'ed, and the name masks
  are then applied to the stat'ed entry.
- An item hidden on one side only is kept, with `filtered = true` on that side (`NodeFlags::LEFT_FILTERED` /
  `RIGHT_FILTERED`), and the pair is still compared. When the filtered side is name-only, the scanner stats it
  after alignment (one `Vfs::stat` per such pair, which is rare) so that both sides have metadata. That side
  is drawn teal.
- A node whose present sides are all filtered has state `Filtered`, also when an error is attached (the side
  flag, the `NodeError` and the warning stay; §1.6.8 item 3). No content test runs for it; quick tests run
  only when both sides were stat'ed (otherwise `diffs` is empty and `time` is `None`). An entry known only by its
  name because its stat failed is filtered when the name masks exclude that name (its kind is unknown: the file
  masks).
- `exclude_protected` (FC-053) marks an entry filtered when its `DosAttrs` contain both SYSTEM and HIDDEN. On
  back-ends without DOS attributes it never applies, so on Linux dotfiles are never excluded by it. In flatten
  mode a folder protected on one side is collected once a folder of the same path was collected on the other
  side (§1.6.8 item 9d).
- Size and date filters (FC-053) apply to files only and need metadata, so they never use the prefilter. An
  entry fails the size filter when `cmp(size, bytes)` is false (`Less`: `size < bytes`; `Greater`:
  `size > bytes`). An entry fails the date filter when `mtime` is missing or `cmp(mtime, cutoff)` is false.
  `DaysAgo{n}` resolves to local midnight of today minus `n` days in `env.now`'s zone.
- *Suppress Filters* (D-44) makes `Filtered` nodes visible (teal); every other node still follows the display
  filter (§2.7).

| # | Filter | Item | Verdict |
|---|---|---|---|
| 1 | (empty) | any | included |
| 2 | `*.c` | F `a.c`, F `x/y.c`; F `a.h` | included; excluded (name-only, D-45) |
| 3 | `-obj\` | D `obj`, D `x/obj` (and their whole subtrees) | excluded, not listed; the folder entries are name-only |
| 4 | `src\` | F `top.txt` (base) | included (base files are always subject to file masks only) |
| 5 | `src\` | D `src`, F `src/a/b.c` | included (subtree rule) |
| 6 | `src\` | D `lib` | PassThrough (`src\` is simple, so a `lib/src` could exist); F `lib/x.c` is excluded |
| 7 | `.\src\` | D `lib` | Excluded (anchored mask, `lib` is not a prefix) |
| 8 | `src\core\` | D `src` / D `src/core` / F `src/a.c` | PassThrough / Included / excluded |
| 9 | `*.c;-test\` | F `test/a.c` | excluded (parent excluded) |
| 10 | `-*.bak` with `align_extensions`: left `x.bak` ↔ right `x.txt` | node | both shown; left `filtered = true` (teal; stat'ed after alignment), the pair compared normally |
| 11 | `size:>1MB` | F `big.bin` (2 MiB) | stat'ed, then filtered (size and date filters never prefilter) |

### 2.4 Timestamp comparison (FC-021; D-28, D-52)

```
compare_times(l, r, rule) -> TimeRelation          // only called when rule.enabled; l, r: FileTime
  if rule.offset = Some(TzOffset{side, hours}): add hours × 3600 s to that side's time (l or r)
  d = r − l in nanoseconds (i128)
  T = rule.tolerance_secs × 10^9
  if |d| ≤ T                                           → Same
  if rule.ignore_timezone:
      if ∃ k ∈ 1..=26: | |d| − k × 3600×10^9 | ≤ T     → Same
  else if rule.ignore_dst:
      if | |d| − 3600×10^9 | ≤ T                        → Same
  if d > 0 → RightNewer else LeftNewer
```

If `rule.enabled` is false, or either mtime is missing (including name-only entries), the criterion is not
applied to that pair (`time = None`); a missing mtime on a stat'ed entry adds one `capability_missing` warning
per base side. Properties (proptest, §6.2.6):
symmetry (`compare(l,r) = mirror(compare(r,l))`), reflexivity (`compare(t,t) = Same`), tolerance
monotonicity (`Same` under T₁ implies `Same` under T₂ ≥ T₁), DST ⊂ TZ (`Same` with `ignore_dst` implies `Same` with
`ignore_timezone`).

| # | Left | Right | tol | DST | TZ | Result |
|---|---|---|---|---|---|---|
| 1 | 12:00:00 | 12:00:02 | 2 | – | – | Same (inclusive) |
| 2 | 12:00:00 | 12:00:02.000000001 | 2 | – | – | RightNewer |
| 3 | 12:00:00 | 11:59:58 | 2 | – | – | Same |
| 4 | 12:00:00 | 13:00:00 | 2 | ✓ | – | Same |
| 5 | 12:00:00 | 13:00:03 | 2 | ✓ | – | RightNewer |
| 6 | 12:00:00 | 11:00:01 | 2 | ✓ | – | Same |
| 7 | 12:00:00 | 14:00:00 | 2 | ✓ | – | RightNewer (DST covers exactly 1 h) |
| 8 | 12:00:00 | 17:00:01 | 2 | – | ✓ | Same (k = 5) |
| 9 | 12:00:00 | 12:30:00 | 2 | – | ✓ | RightNewer (half-hour offsets are not covered: FC-021 says 1 h) |
| 10 | 12:00:00 | +27 h | 2 | – | ✓ | RightNewer (\|k\| ≤ 26) |
| 11 | 12:00:00 | 12:00:00.5 | 0 | – | – | RightNewer (nanosecond resolution) |
| 12 | 12:00:00 | 14:00:01, offset `left+2` | 2 | – | – | Same (left shifted to 14:00:00) |
| 13 | 12:00:00 | 18:00:00 | 2 | – | – | `enabled = false`: not compared (`time = None`) |

### 2.5 Name alignment (FC-010, FC-013, FC-014; D-20)

#### 2.5.1 Keys

For an entry with name `n`, after symlink resolution when `follow_symlinks` is set:

- `kind_class`: Folder | File | Symlink (unfollowed) | Other. Classes never align with each other (D-20).
- `k1` = exact OS name (bytes or UTF-16, via `FileName` equality).
- `k2` = `fold(n)` if case-insensitive, else `n`.
- `k3` = `nfc(k2)` if `align_normalization`, else `k2`.
- `k4` = the stem of `k3` (`FileName::stem` semantics) if `align_extensions` **and** the entry is a file,
  else `k3`.
- *Amended (§1.6.8 item 15):* for a name that is not valid Unicode, `k2` folds the exact OS form (the valid
  UTF-8 runs are folded, the other bytes kept), never the lossy text, and `k3 = k4 = k2`; two different such names
  never share a key.

#### 2.5.2 Pairing (deterministic)

The levels are L1 (`k1`), L2 (`k2`, only if case-insensitive), L3 (`k3`, only if `align_normalization`) and
L4 (`k4`, only if `align_extensions`). At each level, only the entries still unpaired are considered. For
every `(kind_class, key)` group with left list `A` and right list `B`, each sorted by `(k1 bytes)`, pair
`A[i] ↔ B[i]` for `i < min(|A|, |B|)`. Leftovers go to the next level. After L4, the leftovers are orphans.

| # | Left names | Right names | Options | Pairs (orphans) |
|---|---|---|---|---|
| 1 | `a.txt` | `a.txt` | – | `a.txt↔a.txt` |
| 2 | `README` | `readme` | insensitive | `README↔readme` |
| 3 | `README` | `readme` | sensitive | (`README` L, `readme` R) |
| 4 | `A`, `a` | `a` | insensitive (case-sensitive FS on the left) | L1: `a↔a`; `A` left orphan |
| 5 | `café` (NFC) | `café` (NFD) | normalization | `café↔café` |
| 6 | `café` (NFC) | `café` (NFD) | – | two orphans (FC-010 option off) |
| 7 | `x.txt` | `x.csv`, `x.md` | extensions | `x.txt↔x.csv`; `x.md` right orphan |
| 8 | file `x` | folder `x` | any | not aligned: two orphan rows, folder first |
| 9 | symlink `l` (unfollowed) | file `l` | follow off | not aligned |

#### 2.5.3 Flatten (FC-014)

Every file of every subfolder is collected per side, together with its containing `RelPath`. Alignment uses a
level L0 first: exact `(dir, k1)`. Then L1–L4 use names only. Within a group, entries are sorted by
`(dir, k1)`. Folders do not appear in the flattened tree; each side's `EntryView::dir` keeps that side's
containing folder (FC-014 Path column). *Amended (§1.6.8 item 9):* the files are inserted as one batch after every
folder was collected; a folder that could not be listed is a one-sided Folder row (`Failed`, Error), and the other
side's files at or below its path are not aligned but inserted as Unknown rows.

#### 2.5.4 Canonical child order

Children are sorted by:

1. `kind_rank`: Folder first, then File, Symlink, Other;
2. `fold(nfc(representative name))`, where the representative is the left name if present, else the right name;
3. the representative's raw bytes;
4. the right name's raw bytes.

This order is total (INV-FOLDER-3) and independent of listing order and thread count.

### 2.6 Folder states, counters, colours, statistics (FC-020…FC-031, FC-064; D-25, D-48, D-55, D-57)

#### 2.6.1 Quick tests (pure, `quick_tests`)

For two present file-like entries (File, Symlink, Other) of the same kind, both stat'ed (name-only entries skip
every quick test). *Amended (§1.6.8 item 5, maintainer decision):* folder pairs are not quick-tested — a folder is
compared by aggregating its contents (§2.6.4), so its `diffs` are empty and its `time` is `None`.

```
diffs = ∅; time = None
size:        criteria.size && kind == File && l.size ≠ r.size                         → SIZE
timestamps:  rule.enabled && both mtimes present:
                 time = compare_times(l.mtime, r.mtime, rule); time ≠ Same             → TIME
case:        criteria.filename_case && l.name ≠ r.name && eq_folded(l.name, r.name)   → NAME_CASE
             (a name that is not valid Unicode is folded on its exact OS form, §2.5.1)
attributes:  !criteria.dos_attributes.is_empty() && both Attrs::Dos &&
                 (l.attrs ^ r.attrs) & criteria.dos_attributes ≠ 0                     → ATTRIBUTES
unix:        both Attrs::Unix: permissions → mode ≠; owner → uid ≠; group → gid ≠      → PERMISSIONS/OWNER/GROUP
symlinks:    both unfollowed symlinks && l.link_target ≠ r.link_target                 → LINK_TARGET
```

A criterion that cannot be evaluated (missing capability) is skipped, and a `capability_missing` warning is
recorded once per (criterion, side).

#### 2.6.2 Content test and state (FC-025…FC-027; D-25, D-48)

```
quick_same  = diffs.is_empty()
run_content = method ≠ None && kind == File && no side filtered && !(skip_if_quick_same && quick_same)
              (method ≠ None && skip_if_quick_same && quick_same → flag CONTENT_SKIPPED)
if run_content:
    guard   = budget.acquire(min(size_l + size_r, budget_capacity), cancel)    // Err(Cancelled) → state Unknown
    content = comparator(method).compare(VfsSource(l), VfsSource(r), ctl)       // Err(Cancelled) → state Unknown
    drop(guard)
    outcome warnings contain rules_size_limit → flag RULES_SKIPPED_SIZE

OVERRIDABLE = { TIME, SIZE }                                                    // D-48
state = match content:
  None                                     → base(diffs, time)
  Some(Error)                              → Error
  Some(BinarySame | RulesSame)             → override_quick ? base(diffs − OVERRIDABLE, None) : base(diffs, time)
  Some(UnimportantOnly | ImportantDifferences | BinaryDifferent)
                                           → diffs |= CONTENT; TIME ∈ diffs ? newer(time) : Different
base(d, t) = d.is_empty() ? Same : (TIME ∈ d ? newer(t) : Different)
newer(t)   = LeftNewer when t == LeftNewer, RightNewer when t == RightNewer
```

The stored `diffs` keep every flag found, including those an override cancelled (columns and JSON show them).

| # | Quick result | Content | override | State |
|---|---|---|---|---|
| 1 | same | not requested | – | Same |
| 2 | TIME (left newer) | not requested | – | LeftNewer |
| 3 | SIZE | not requested | – | Different |
| 4 | TIME | BinarySame | on | Same (diffs keep TIME for display) |
| 5 | TIME | BinarySame | off | LeftNewer |
| 6 | same | BinaryDifferent | – | Different (+CONTENT) |
| 7 | TIME (right newer) | ImportantDifferences | – | RightNewer (+CONTENT) |
| 8 | SIZE | RulesSame (CRLF vs LF) | on | Same |
| 9 | same | UnimportantOnly | on | Different (+CONTENT); category Same under ignore-unimportant |
| 10 | any | Error | – | Error |
| 11 | TIME + ATTRIBUTES | BinarySame | on | Different (D-48: attributes are not overridden) |
| 12 | NAME_CASE | RulesSame | on | Different (D-48) |
| 13 | SIZE + PERMISSIONS | RulesSame | on | Different (D-48) |
| 14 | TIME (left newer) | UnimportantOnly | on | LeftNewer (+CONTENT); category Same under ignore-unimportant |
| 15 | TIME (left newer) | UnimportantOnly | off | LeftNewer (+CONTENT); category LeftNewer under ignore-unimportant |

Other nodes (first match wins; *amended*, §1.6.8 item 3): a node whose present sides are all filtered →
`Filtered`, also with an error attached; a per-entry stat error on either side → `Error` (flag
`LEFT_ERROR`/`RIGHT_ERROR`); a child of a failed listing → `Unknown`; a node with one side only →
`LeftOrphan`/`RightOrphan`. When a folder listing fails on one side, the folder gets `ScanStatus::Failed` and
state `Error`, and the other side's entries are inserted as children with state `Unknown` (they are not orphans:
the failing side is unknown) and are not scanned further. Flatten mode does the same with the files at or below
the failed folder's path (§2.5.3).

#### 2.6.3 Category (display) function

```
category(n, iu):
  n.state == Filtered                                        → Filtered
  n is a folder                                              → own_category(n, iu)            (§2.6.4)
  iu && n.content.kind == UnimportantOnly                    → the category of state_if_rules_same(n)
  otherwise                                                  → the state's same-named category
state_if_rules_same(n) = override_quick ? base(diffs − OVERRIDABLE − CONTENT, None)
                                        : base(diffs − CONTENT, time)
```

Ignore-unimportant only ever makes a category "more same": a node that is a difference under `iu = true` is a
difference under `iu = false` (INV-FOLDER-10).

#### 2.6.4 Counters, folder state and colours (D-55)

Every folder holds `Aggregates`: for each `iu ∈ {false, true}`, the counts of its **descendants** (the folder
itself excluded). Contributions:

```
file-like node n (File, Symlink, Other):   files[category(n, iu)]       += 1
folder node F (not the root):               folders[own_category(F, iu)] += 1

own_category(F, iu):                                  (order amended in wave 3, §1.6.8 item 2)
  every present side filtered              → Filtered  (also an orphan)
  ScanStatus::Failed                       → Error     (also with one side only: a failure is never an orphan)
  F is a child of a failed listing         → Unknown
  F has one side only                      → LeftOrphan / RightOrphan
  ScanStatus::NotScanned                   → Unknown   (pending, cancelled, or an orphan folder not scanned)
  scanned pair, a = F.aggregates.get(iu):
      a.files.differences() + a.folders.differences() > 0   → Different
      else a.files.error + a.folders.error > 0              → Error
      else a.files.unknown + a.folders.unknown > 0          → Unknown
      else                                                  → Same
folder canonical state = own_category(F, false) as a CompareState
```

Maintenance (inside the write lock that publishes a change): the change yields a delta (contributions removed
and added, for both `iu` values). For each ancestor from the parent up to the root: apply the delta to its
counters, recompute its own category, and when that changed, add the change (old → new own category) to the
delta applied to the remaining ancestors. Listing a folder adds the batch of its children's contributions in
one pass. Cost: O(depth) per change (O(depth²) in the worst case when every ancestor changes category).
`Pair<SideTotals>` (present, non-filtered entries; bytes of files only) is maintained the same way when entries
are inserted. INV-FOLDER-5: every stored counter and total equals a recount.

Colour classes (FC-031, `TreeRead::colour(node, side, iu)`, O(1)). For folders `c` is the field-wise sum
`a.files + a.folders` with `a = aggregates.get(iu)`; "this side's newer" is `c.left_newer` for Left and
`c.right_newer` for Right, and "the other side's newer" is the opposite counter.

| Condition (first match wins) | ColourClass |
|---|---|
| no entry on `side` | Absent |
| `side` filtered | Filtered |
| file: category Same / Different / Error | Same / NewerOrDifferent / Error |
| file: LeftNewer (side Left / Right) | NewerOrDifferent / OlderOrUnknown |
| file: RightNewer (side Right / Left) | NewerOrDifferent / OlderOrUnknown |
| file: LeftOrphan, RightOrphan | Orphan |
| file: Unknown | OlderOrUnknown |
| folder: `Failed` / a child of a failed listing / one side only (in this order, §1.6.8 item 2) | Error / OlderOrUnknown / Orphan |
| folder: `c.different > 0` or this side's newer > 0 | NewerOrDifferent ("contains newer/different") |
| folder: `c.left_orphan + c.right_orphan > 0` | Orphan ("contains orphans") |
| folder: the other side's newer > 0, `c.unknown > 0`, or `NotScanned` | OlderOrUnknown ("contains older") |
| folder: `c.error > 0` | Error |
| folder: `c.filtered > 0 && c.total() == c.filtered` | Filtered ("only filtered") |
| otherwise | Same |

#### 2.6.5 Statistics and verdict (O(1) from the root counters)

```
stats(iu) = FolderStats { files: root.aggregates.get(iu).files, folders: root.aggregates.get(iu).folders,
                          left/right: root totals, content: ContentCounts }
ContentCounts: one count per content result kind of the nodes that ran a content test;
               not_compared = number of nodes with flag CONTENT_SKIPPED
verdict(iu):
  D(x)       = root.aggregates.get(x).files.differences() + root.aggregates.get(x).folders.differences()
  incomplete = !is_complete() || (unknown + error counts of files and folders in root.aggregates.get(iu)) > 0
  D(true) > 0                → Differences
  else incomplete            → Incomplete
  else !iu && D(false) > 0   → UnimportantOnly
  else                       → NoDifferences
```

`D(true) > 0` is exactly "some node is an important difference": by INV-FOLDER-10 a difference under `iu = true`
is a difference under both values, and Unknown/Error categories do not depend on `iu`. With
`--ignore-unimportant` the verdict is never UnimportantOnly (12 → 2, D-01/D-02).

#### 2.6.6 The `criteria` vocabulary (D-57)

```
Criteria::parse_list(list):
  items: split on ',' and ASCII whitespace; empty items ignored; words match ASCII-case-insensitively
  start: size = false, timestamps_enabled = false, tolerance_secs = 2, ignore_dst = false,
         ignore_timezone = false, tz_offset = Clear, filename_case = false, dos_attributes = ∅,
         unix = {false, false, false}, content_method = None          (all as Some(…) in the patch)
  size                                   → size = true
  timestamp[:<n>sec][;IgnoreDST]         → timestamps_enabled = true; tolerance_secs = n; ignore_dst when given
  timezone:ignore                        → timestamps_enabled = true; ignore_timezone = true
  timezone:(left|right)(+|-)<0..12>      → timestamps_enabled = true; tz_offset = Set(TzOffset{side, ±hours})
  attrib:<letters>                       → dos_attributes = DosAttrs::parse_letters(letters)
  owner | group | permissions            → the unix field
  filename-case                          → filename_case = true          (Hikasa extension for FC-022)
  crc | binary | rules-based | rules     → content_method (a second method → InvalidRules)
  follow-symlinks                        → patch.follow_symlinks = Some(true)
  ignore-unimportant                     → spec.ignore_unimportant = true
  version                                → InvalidRules("criterion `version` (FC-028) is not available in this version")
  anything else                          → InvalidRules("unknown criterion `<word>`")
```

Only these spellings are accepted (one vocabulary for the CLI and scripts, T11): the tolerance unit is `sec`, the
offset sides are `left` and `right`; `timestamp:3`, `timestamp:3s`, `timestamp:3secs`, `timezone:lt+2` are
`InvalidRules` (§1.6.8 item 14).

| # | `--criteria` | Effect |
|---|---|---|
| 1 | `size` | size only; timestamps off; no content test |
| 2 | `size,timestamp:3sec;IgnoreDST` | size; timestamps with 3 s tolerance and DST |
| 3 | `timestamp timezone:left+2 crc` | timestamps (2 s) with the left side shifted by +2 h; CRC content |
| 4 | `rules-based ignore-unimportant` | rules content only; display ignores unimportant |
| 5 | `attrib:HRS,owner` | DOS H/R/S subset and owner |
| 6 | `crc,binary` | `InvalidRules` (two methods) |
| 7 | `version` | `InvalidRules` (FC-028 is P1) |

### 2.7 Folder display filters (FC-055, FC-056, FC-057; D-19, D-44)

| Preset | SAME | LEFT_ORPHANS | RIGHT_ORPHANS | LEFT_NEWER | RIGHT_NEWER | DIFFERENCES |
|---|---|---|---|---|---|---|
| All | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Differences | | ✓ | ✓ | ✓ | ✓ | ✓ |
| Same | ✓ | | | | | |
| NoOrphans | ✓ | | | ✓ | ✓ | ✓ |
| DifferencesNoOrphans | | | | ✓ | ✓ | ✓ |
| Orphans | | ✓ | ✓ | | | |
| LeftNewer | | | | ✓ | | |
| RightNewer | | | | | ✓ | |
| LeftNewerAndLeftOrphans | | ✓ | | ✓ | | |
| RightNewerAndRightOrphans | | | ✓ | | ✓ | |
| LeftOrphans | | ✓ | | | | |
| RightOrphans | | | ✓ | | | |
| None | | | | | | |

The atom of a category is: Same → SAME, LeftOrphan → LEFT_ORPHANS, RightOrphan → RIGHT_ORPHANS,
LeftNewer → LEFT_NEWER, RightNewer → RIGHT_NEWER, Different/Unknown/Error → DIFFERENCES (D-19). `Filtered` has
no atom: it is governed by Suppress Filters only (D-44).

```
accepts(c)         = toggles ∋ atom(c)
file_visible(n)    = cat == Filtered ? suppress_filters : accepts(cat)            cat = category(n, iu)
folder_visible(F)  = cat == Filtered ? suppress_filters : match mode {           cat = own_category(F, iu)
                       AlwaysShowFolders => true,
                       CompareStructure  => accepts(cat) || visible_below(F, files + folders),
                       OnlyFiles         => visible_below(F, files) }
visible_below(F, s) = Σ_{c : accepts(c)} a.s[c] > 0  ||  (suppress_filters && a.s.filtered > 0)
                      with a = F.aggregates.get(iu)
visible_rows:       pre-order over children in canonical order; the root is not listed; a node is listed iff it is
                    visible and its parent is listed (the root counts as listed) and the parent is expanded
                    (`expanded(parent)`; the root is always expanded); depth of the root's children = 0
```

Because every counted descendant that is accepted makes each of its ancestors visible under
`CompareStructure` (and every accepted descendant file does so under `OnlyFiles`), `visible_below` equals the
recursive "has a visible child" definition, at O(1) per folder (INV-FOLDER-9). Folder visibility on a partial
tree is the same function of the current counters.

CLI vocabulary (`--display`): `all`, `differences`, `same`, `no-orphans`, `differences-no-orphans`, `orphans`,
`left-newer`, `right-newer`, `left-newer-orphans`, `right-newer-orphans`, `left-orphans`, `right-orphans`,
`none`. The script spellings `display-…` and `display-mismatches` (= differences) are also accepted (SCR-001).

### 2.8 Encoding detection and decoding (TC-001, FF-032; D-42)

```
detect(bytes, setting, opts):
 1. opts.override_encoding = Some(e)  → (e, bom_len = |BOM| if bom_sniff matches e else 0, Override)
 2. bom_sniff(bytes) = Some(e, n)     → (e, n, Bom)
      order: EF BB BF → UTF-8; FF FE 00 00 → UTF-32LE; 00 00 FE FF → UTF-32BE; FF FE → UTF-16LE; FE FF → UTF-16BE
      (UTF-32LE is tested before UTF-16LE because FF FE is its prefix)
 3. setting = Fixed{label}            → (for_label(label), 0, Fixed)   // label validated at format compile time
 4. setting = Auto{fallback}:
    a. utf_pattern(bytes[..min(len, 4096)]) = Some(e)                        → (e, 0, UtfPattern)
    b. no byte ≥ 0x80 in bytes (whole content, vectorised `is_ascii`):
         fallback is Some and its encoding is ASCII-compatible             → (fallback, 0, FormatFallback)
         else opts.locale_fallback is ASCII-compatible                      → (locale, 0, LocaleFallback)
         else                                                               → (UTF-8, 0, Ascii)
    c. std::str::from_utf8(bytes) is Ok                                     → (UTF-8, 0, Utf8Valid)
    d. mostly_utf8(bytes) (whole content): V = valid multi-byte sequences (chars ≥ U+0080),
       M = malformed sequences (maximal subparts, exactly the units `decode` replaces; a truncated sequence at
       the end counts); V ≥ 1 && M ≤ 16 && 8 × M ≤ V                       → (UTF-8, 0, Utf8Lossy)
    e. chardetng: d = EncodingDetector::new(Iso2022JpDetection::Deny);
       w = bytes[..min(len, opts.detect_limit)]; d.feed(w, w.len() == len); d.guess(None, Utf8Detection::Deny)
                                                                            → (guess, 0, Detector)

utf_pattern(s):                              // shared with binary::sniff (one function, D-42)
  UTF-32: U = ⌊|s| / 4⌋ units (U ≥ 2). LE when ≥ 90 % of units have bytes [x, y, 0, 0] with (x, y) ≠ (0, 0)
          and every unit is a Unicode scalar value (≤ 0x10FFFF, not a surrogate); BE mirrored ([0, 0, y, x]).
  UTF-16: P = ⌊|s| / 2⌋ pairs (P ≥ 2); ZO = zero bytes at odd offsets, ZE = zero bytes at even offsets.
          ZO × 100 ≥ 40 × P && ZE × 100 ≤ 5 × P → UTF-16LE; ZE × 100 ≥ 40 × P && ZO × 100 ≤ 5 × P → UTF-16BE.
  UTF-16, one alphabet (BOM-less non-Latin text mixed with ASCII), LE shown (BE: high and low bytes swap):
          H[v] = units whose high (odd-offset) byte is v; v* = the most frequent v ≠ 0 (the smallest on a tie).
          v* ∈ 0x01..=0x2F && H[v*] ≥ 30 % P && H[0] ≥ 5 % P && H[0] + H[v*] ≥ 90 % P && ZE ≤ 5 % P → UTF-16LE.
  Candidates in order: UTF-32LE, UTF-32BE, UTF-16LE, UTF-16BE, one-alphabet LE, one-alphabet BE. A candidate
  is accepted only when its decoded sample is plausible text: bad × 10 ≤ units, where bad counts control
  characters other than HT, LF, VT, FF, CR, BS, SUB and ESC (C0 plus DEL and C1 U+0080–U+009F) and unpaired
  surrogates. UTF-32 is tested first (ASCII UTF-32 also has many zero bytes at odd offsets).

decode(bytes, enc): strip bom_len; WHATWG encodings use encoding_rs `Decoder::decode_to_string_without_replacement`
  in a loop that counts every `DecoderResult::Malformed` and inserts U+FFFD; UTF-32 uses Hikasa's decoder
  (4-byte units; invalid scalar or 1–3 trailing bytes → one U+FFFD each, counted).
```

Why this order: ASCII-range UTF-16 and UTF-32 text is valid UTF-8 (its NUL bytes are valid), so the pattern
test must run before the ASCII and UTF-8 tests; the docs order "BOM → chardetng → format default → current
locale" is kept for content with non-ASCII evidence, and the fallbacks apply only to content without it.
chardetng 1.0's `find_score` is gated behind a test-only feature, so a plausible format fallback cannot be
preferred over the detector's guess (D-42).

Why step d (amended in the wave-2 review): one stray byte in an otherwise UTF-8 file (a Latin-1 "ï", a
truncated last char) used to send the whole file to chardetng with UTF-8 denied, so every non-ASCII char of
every line came out as mojibake. Valid multi-byte UTF-8 sequences are strong evidence of UTF-8: legacy
single-byte text almost never forms them (an accented letter is followed by an ASCII byte, not by a byte in
0x80–0xBF), and nearly every non-ASCII byte of legacy text is malformed UTF-8. The ratio and the cap keep
legacy files with an accidental valid sequence on the detector path.

Why the plausibility check (wave-2 review): arrays of small 16- or 32-bit integers have the zero-byte pattern
of ASCII UTF-16/32 but decode to control characters; without the check they were "text" (D-12 bypassed). The
one-alphabet rule recognises BOM-less UTF-16 Cyrillic, Greek, Hebrew, Arabic and similar text, which used to be
pure ASCII bytes with NULs (decoded as garbage, sniffed binary). Limitation: BOM-less UTF-16 in scripts whose
high bytes spread over many values (CJK, Hangul) and BOM-less non-Latin UTF-32 are still not recognised; they
need a BOM or an encoding override.

`malformed > 0` sets the warning `lossy_decode`. INV-ENC-1: decoding is total, never panics, and for UTF-8,
UTF-16 and UTF-32 `decode(encode(s)) == s` (property test with encoding_rs encoders and Hikasa's UTF-32 encoder).

| # | Content (setting Auto, locale UTF-8) | Result |
|---|---|---|
| 1 | `EF BB BF 61` | UTF-8, BOM 3, `bom` |
| 2 | `FF FE 00 00 61 00 00 00` | UTF-32LE, BOM 4, `bom` |
| 3 | `61 00 62 00 0D 00 0A 00` (BOM-less ASCII UTF-16LE) | UTF-16LE, `utf_pattern` |
| 4 | `00 61 00 62 00 0D 00 0A` (BOM-less ASCII UTF-16BE) | UTF-16BE, `utf_pattern` |
| 5 | `61 00 00 00 62 00 00 00` (BOM-less UTF-32LE) | UTF-32LE, `utf_pattern` |
| 6 | `hello` with format fallback `windows-1252` | windows-1252, `format_fallback` |
| 7 | `hello`, no fallback | UTF-8 (the locale), `locale_fallback` |
| 8 | `caf\xC3\xA9` | UTF-8, `utf8_valid` |
| 9 | a 1 KiB French sample in windows-1252 (`caf\xE9 …`) | windows-1252, `detector` |
| 10 | a 1 KiB Japanese sample in Shift_JIS | Shift_JIS, `detector` |
| 11 | any content with override KOI8-R | KOI8-R, `override` |
| 12 | Cyrillic UTF-8 plus one `FF` byte | UTF-8, `utf8_lossy`, 1 malformed |
| 13 | French UTF-8 with one windows-1252 `EF` byte | UTF-8, `utf8_lossy`, 1 malformed |
| 14 | windows-1252 text with one accidental valid UTF-8 sequence (1 valid, 6 malformed) | windows-1252, `detector` |
| 15 | BOM-less UTF-16LE Cyrillic text | UTF-16LE, `utf_pattern` |

### 2.9 Binary detection (TC-003)

```
sniff(bytes):
  empty                                       → text   (Empty)
  bom_sniff(bytes).is_some()                  → text   (Bom)
  magic(bytes) = Some(kind)                   → BINARY (Magic{kind})
  w = first SNIFF_LEN (8192) bytes
  utf_pattern(w[..min(|w|, 4096)]).is_some()  → text   (UtfPattern)           // the same function as §2.8
  memchr(0x00, w) = Some(off)                 → BINARY (NulByte{off})
  ctrl = #bytes in w with b < 0x20 && b ∉ {09 HT, 0A LF, 0B VT, 0C FF, 0D CR, 08 BS, 1A SUB, 1B ESC}, plus #(b == 0x7F)
  ctrl × 1000 > 100 × |w|                     → BINARY (ControlRatio{permille = ctrl×1000/|w|})
  otherwise                                   → text   (Text)

magic(bytes) at offset 0 (in-house table replacing `infer`, §0.7):
  PNG  89 50 4E 47 0D 0A 1A 0A      GIF   "GIF87a" | "GIF89a"       JPEG  FF D8 FF
  PDF  "%PDF-"                      ZIP   50 4B 03 04 | 50 4B 05 06 | 50 4B 07 08
  gzip 1F 8B                        ELF   7F 45 4C 46               7z    37 7A BC AF 27 1C
  xz   FD 37 7A 58 5A 00            zstd  28 B5 2F FD
```

Binary content does not stop a text comparison. The document is decoded (lossy if needed), the warning
`binary_content` is added, and the verdict becomes binary (§2.17.5). Rules-based quick compare and
`RulesComparator` use binary comparison (D-12). Fixtures (§6.2.4): each magic kind, BOM-less UTF-16LE/BE and
UTF-32LE text, BOM-less UTF-16LE Cyrillic text, arrays of small 16- and 32-bit integers (binary: the
plausibility check of `utf_pattern` rejects them, then the NUL rule applies), text with ESC sequences, an 11 %
control-byte buffer.

### 2.10 Tokenizer (FF-020…FF-027; D-13, D-41, D-58)

#### 2.10.1 Definitions

A *line* is the content without its terminator. Positions are byte offsets on char boundaries.
`is_word(c) = c.is_alphanumeric() || c == '_' || format.text.word_chars.contains(c)` (D-58), and whitespace
is `char::is_whitespace`. The
*leading-whitespace prefix* of a line is its maximal whitespace prefix, and the *trailing-whitespace suffix* is
its maximal whitespace suffix. "EE" means "Everything Else" (`ElementId(0)`).

#### 2.10.2 Per-line algorithm

```
tokenize_line(line, line_no, state) -> (tokens, end_state)
  p = 0
  // (a) continuation of a multi-line construct
  match state:
    InLines{item, remaining}:
        emit whole-line token of item, trimmed per D-13 (leading/trailing ws → EE tokens)
        return remaining > 1 ? InLines{item, remaining − 1} : Normal
    InDelimited{item}:
        lead = len(leading-ws prefix); emit EE[0, lead) if lead > 0          // D-13 continuation trimming
        match find_close(item, line, lead):
          Some(end) → emit item[lead, end); p = end                           // continue scanning in Normal
          None      → t = max(lead, start of trailing-ws suffix); emit item[lead, t); emit EE[t, len) if t < len
                      return InDelimited{item}
  // (b) left-to-right scan; first matching item in list order wins (FF-026)
  while p < len:
    if let Some(m) = first_match(line, p, line_no):
        flush pending EE run; emit item[p, m.end)
        if m.eol_terminated:                        // the token ended because the line ended (D-13)
            emit EE[m.end, len) if m.end < len      // the trailing-whitespace suffix
            return m.next_state                     // Normal, InDelimited{item} or InLines{…}
        p = m.end
    else:
        extend pending EE run by one char; p += char_len
  flush pending EE run; return Normal
```

`first_match(line, p, line_no)` tries the items in order. A zero-length match counts as **no match**
(INV-TOK-1). The rules per kind:

| Kind | Matches at `p` when | Token end | Next state |
|---|---|---|---|
| Basic | literal `text` starts at `p` (case per `match_case`), or the regex match found by `find_at(line, p)` starts at `p` | end of match | Normal |
| Delimited | `from` matches at `p` (literal or anchored regex) | `find_close(item, line, end_of_from)`; if `None`: start of the trailing-ws suffix (but ≥ end of `from`), D-13 | Normal if closed, if `to` is None, or if `stop_at_eol`; else `InDelimited{item}` |
| List | some token matches at `p` with word boundaries (below); the **longest** wins | end of match | Normal |
| Columns | the char column of `p` (0-based) equals `from_column − 1` | byte offset of char column `to_column` (inclusive), or EOL trimmed per D-13 when `to_column` is None; clamped to `len` | Normal |
| Lines | `p == 0` and (`or_line_one && line_no == 0` or `heading` is found anywhere in the line) | EOL (trimmed per D-13) | `InLines{item, next_lines}` if `next_lines > 0`, else Normal |

**Regex items** match at `p` when the leftmost match found by `find_at(line, p)` starts at `p`;
equivalently, when an anchored match at `p` exists, with the same end (the leftmost search returns a match at
`p` whenever one exists). The implementation decides it either way (§2.10.5). A fancy-regex runtime-limit hit
of that decision counts as no match and is counted in `LineOutcome::regex_errors`.

**Field boundaries (Columns items; added in the wave-2 review).** The first column of every Columns item
(`from_column`, when > 1) is a field boundary. Every item sees the line *up to the next boundary* as the whole
line: no token crosses a boundary, `$` and the List word boundaries apply at it, and a token that would run to
the end of the line (an unterminated Delimited item, a Columns item without `to_column`, a Lines item) ends at
the boundary, trimmed per D-13 (the blanks before the boundary are an EE token). `^` and look-behind still see
the real start of the line. The scan then goes on at the boundary, where the Columns item (or an earlier item
in list order) matches. The end state of the line is the non-`Normal` next state of the last
end-of-line-terminated token (so a multi-line construct cut at a boundary still continues on the next line),
or `Normal`. A continuation (step (a)) also ends at the first boundary. Without Columns items (every preset but
COBOL) nothing changes. This is what keeps COBOL's columns 73–80 the Identification element when a literal or
an identifier reaches column 72 (§2.13.3 row 16).

**Word boundaries for List items.** If the matched text starts with a word char, then `p == 0` or
`!is_word(char before p)`. If it ends with a word char, then `end == len` or `!is_word(char at end)`. A List
item made only of literal word tokens is compiled to a hash set. At a word start the maximal word (by
`is_word`, so with COBOL's `word_chars = "-"` the maximal word of `END-OF-FILE` is the whole name) is looked up
once (O(1)), folded if `!match_case`. Other tokens (regex, or containing non-word chars such as `#include`,
`@end`, `<=`, Java's `non-sealed`) are tried individually.

**Element ids (D-41).** `CompiledGrammar` assigns `ElementId(0)` to "Everything Else", then ids 1, 2, … to
element names in order of first use in the `grammar` array, then to names declared in `[elements]` but never
used, in lexicographic order. The `[elements]` table only supplies `case_sensitive` and `important`; its key order
is irrelevant (toml deserialises tables through a `BTreeMap`).

#### 2.10.3 Closing-delimiter search and escapes (FF-021)

```
find_close(item, line, x) -> Option<end>
  while x < len:
    if let Some(e) = item.escape && line[x..] starts with e && x + |e| < len:
        if to is a literal whose first char == e && line[x+|e|..] does not start with e:
            // not an escape: e is the closer's first char and is not doubled → test the closer below
        else:
            x += |e| + len(char at x + |e|); continue                     // skip escape + escaped char
    if to matches at x (literal starts_with / anchored regex): return Some(x + match_len)
    x += char_len(x)
  None
```

| Item | Line | Token | Why |
|---|---|---|---|
| `"` … `"` esc `\` | `s = "a\"b" + c` | `"a\"b"` | `\"` is escaped |
| `"` … `"` esc `\` | `"a\\" x` | `"a\\"` | `\\` is an escaped backslash, then `"` closes |
| `'` … `'` esc `'` (SQL) | `'it''s'` | `'it''s'` | doubled delimiter |
| `"` … `"` esc `"` (VB) | `"say ""hi"""` | the whole literal | doubled delimiter |
| `'` … `'` esc `'` | `'abc' x` | `'abc'` | single `'` followed by a space closes |
| `/*` … `*/` | `x /* a` ⏎ `   b */ y` | line 1: `/* a`; line 2: EE `   `, Comment `b */`, EE ` y` | multi-line plus D-13 trimming |
| `//` → EOL | `x = 1; // c   ` | Comment `// c`, EE `   ` (trailing) | D-13 |
| `"` … `"` stop at EOL | `"abc` | String `"abc` | unterminated, ends at EOL |

#### 2.10.4 Token invariants

- INV-TOK-1: every token is non-empty.
- INV-TOK-2: tokens are sorted, contiguous and cover `[0, len)` exactly.
- INV-TOK-3: boundaries are char boundaries, and EE tokens are never adjacent (they are merged).
- INV-TOK-4: the tokens of a line depend only on `(line, line_no, start state)`.
- INV-TOK-5: `tokenize_all` gives the same result with any pool as the sequential run.

#### 2.10.5 Performance design (normative where it affects results: it must not)

- A first-char prefilter: a 128-bit ASCII bitmap plus an "any" flag per item. Literals contribute their first
  char, in both cases when `!match_case`. Regex items contribute "any" unless their first byte set can be
  computed trivially (a literal prefix). Positions whose char cannot start any item are added to the EE run
  without trying any item. Case folding crosses the ASCII boundary (KELVIN SIGN U+212A folds to `k`, LONG S
  U+017F to `s`), so a case-insensitive set that admits a non-ASCII char also admits `k`, `K`, `s` and `S`.
- A next-match cache per regex item and haystack: an unanchored `find_at(line, p)` gives the leftmost match
  `(s, e)` at or after `p`, which answers every position up to `s` in O(1); "no match after `p`" is cached for
  the rest of the line. The cache is consulted first.
- An exact ASCII accelerator for simple patterns (sequences of character classes, optional groups,
  alternation) decides the anchored question without the engine. Each decision examines at most 512 bytes; a
  decision that examined more than 16 bytes without a match primes the cache instead, so a run is scanned
  once, not once per position.
- Unanchored searches are charged the bytes they cover. When matches are skipped (another item's token covers
  them), later searches cover the same bytes again; once an item's charge on a haystack exceeds
  `2 × len + 4096`, the item switches to anchored attempts, which never look past the match at hand. So each
  item scans a line amortised a bounded number of times (wave-2 review: lines of blanks before a non-blank,
  `${`/`#[` without a closer, and strings containing `rem` used to cost quadratic time in the line length).
  Every built-in preset tokenises any line in time linear in its length (`tests/long_lines.rs` and a stress
  run over all presets with 1 MiB lines of repeated fragments). A user pattern whose anchored attempts fail
  late (for example `x.*y` on a long run of `x`, once the item's search budget is spent) can still cost up to
  the line length per position; the backtrack limit bounds fancy patterns, and cancellation stays responsive.
- `fancy-regex` runtime-limit errors count as "no match" and are counted (`LineOutcome::regex_errors`). They
  surface as warning `regex_runtime_error`, which is deterministic because the backtrack limit counts steps. An
  unanchored search that hits the limit is not counted (the hit lies somewhere after `p`) and is never cached
  as "no match": the item switches to anchored attempts, which count their own hits exactly as the reference
  algorithm (one anchored attempt per position, no fast paths) does.
- Cancellation (§5.3): besides the per-chunk check, `tokenize_all` checks the token inside a line every 64 KiB
  of scanned text and before every regex engine call, so one multi-megabyte line cannot delay a cancel beyond
  G4.

#### 2.10.6 Parallel speculative tokenisation (`tokenize_all` with a pool)

```
chunks of C = 8192 lines; each chunk owns one byte arena and one end-offset vector (LineBytes, §1.4.4).
Phase 1 (parallel): each chunk k is tokenised assuming start state Normal, recording for each line i its
                    speculative start state s_k[i], appending f's output to the chunk arena, and the chunk end
                    state e_k. Chunk 0 starts in the true initial state Normal.
Phase 2 (sequential, k = 1..): actual_k = end state of chunk k−1 after its fix-up.
     if actual_k == Normal: the chunk is correct.
     else: re-tokenise from actual_k line by line, writing the outputs of the re-tokenised prefix into the chunk's
           patch arena, until a line i is reached whose recomputed start state equals s_k[i]. From there on the
           speculative results are exact (INV-TOK-4) and stay in the original arena. If no such line exists, the
           whole chunk is patched and its end state is the recomputed one.
Grammars without multi-line items (`!has_multiline()`) skip phase 2 and store no start states.
```

Correctness: by INV-TOK-4, a line's tokens are a function of its start state. Once the recomputed start state
equals the speculative one, every later line in the chunk sees the same inputs. So the outputs are identical
to the sequential run. Worst case: an unterminated block comment near the top makes every chunk re-tokenise,
which is sequential cost plus one wasted parallel pass.

#### 2.10.7 Line weights (TC-012)

`line_weight(line)` = the maximum `priority` over the weights whose pattern is found anywhere in the raw line
(substring with case per `match_case`, or regex search). 0 when none matches. Weights are computed (with the
same chunked parallelism as keys) only when the algorithm resolves to Standard, the only engine that uses them.
A weight regex whose search hits the fancy-regex runtime limit counts as not found;
`line_weight_with_errors(line) -> (u8, u32)` also returns those hits, which hikasa-text adds to `regex_errors`
(warning `regex_runtime_error`; §5.5 rule 3).

#### 2.10.8 External conversion command lines (FF-030; D-39)

- Tokens are separated by unquoted ASCII whitespace.
- `"…"` groups a token; the quotes are removed, and `""` inside quotes is a literal `"`.
- Backslash is **not** an escape, so Windows paths survive.
- After splitting, `%s`, `%t` and `%o` are substituted inside tokens, so substituted values are never re-split.
  `%%` → `%`. Any other `%x` → `Template` error.
- An empty template or an empty `argv[0]` → `Template` error.
- The program runs without a shell, with stdin null and stdout/stderr discarded.
- Success requires exit status 0 **and** a non-empty `%t` file. Temporary files are removed on every path.
- Cancellation and the timeout kill the child.
- Temporary files (wave-2 review): each conversion creates its own directory
  `<temp_dir>/hikasa-conv-<16 hex digits>` exclusively (`mkdir` fails on any existing entry, a symbolic link
  included, so nothing pre-planted in a shared temporary folder is followed) with mode 0700 on Unix; `%s` is
  `hikasa-conv.src` and `%t` is `hikasa-conv.out` inside it. The name hashes injected data (temporary folder,
  original name, input length and first bytes) with the process id and a per-process counter: hard to guess,
  with no clock and no randomness (T1); a taken name is skipped. The directory is removed with everything in
  it after the runner has reaped the child, on every path (retried when something wrote into it meanwhile).
- Limitation: only the direct child is killed. Killing its descendants needs a process-group kill, an OS API
  that no approved crate provides (§0.7). A descendant that outlives the child cannot write `%t` afterwards,
  because the directory is gone; it may keep running until it ends by itself.

### 2.11 Regex engine selection (FF-020…FF-022, TC-020)

```
Pattern::compile(src, ci, limits):
  1. regex::RegexBuilder::new(src).case_insensitive(ci).size_limit(limits.size_limit).build()   → Standard
  2. on error e1: fancy_regex::RegexBuilder::new(&with_ci(src, ci))          // "(?i)" prefix when ci
                    .backtrack_limit(limits.backtrack_limit).delegate_size_limit(limits.size_limit).build() → Fancy
  3. on error e2: PatternError { message: "<e1>; <e2>" }
```

Linear-time `regex` is always preferred. `fancy-regex` is reached only for patterns that `regex` rejects
(backreferences, look-around). Both engines see the whole line as the haystack. `find_at(line, p)` therefore
evaluates `^`, `$` and `\b` against the real line context. `^` means line start.

### 2.12 Format detection, shadowing, registry loading (FF-001…FF-004)

#### 2.12.1 Detection

```
detect(file_name, overrides):
  for entry in entries (in order):
     enabled = overrides.0.get(id).copied().unwrap_or(entry.enabled)
     if !enabled || entry.mask.is_empty(): continue
     if (∃ include item matching file_name) && (no exclude item matches file_name):
         return Mask{ id, mask: first matching include item's source }
  return Fallback                                         // "Everything Else"
```

Format masks match names only, case-insensitively (D-10). A format mask that contains a separator is a
`FormatError::Invalid` at load time. The winning format is compiled lazily (cached per entry). A compile
error is returned to the caller, never silently skipped (T9).

#### 2.12.2 Forced format

`--format <x>` → `find(x)`: an exact id match, else a case-insensitive name match, else `UnknownFormat`. The
reason is `Forced`.

#### 2.12.3 Shadowing (sound)

An enabled entry E with a non-empty mask is `Shadowed { by }` iff every include item `i` of E is covered by
some include item `j` of an earlier enabled entry E′ that has **no** exclude items. `by` lists those E′ in
order. `i.is_covered_by(j)` returns true only when one of these holds:

- (a) `j` is `*` or `*.*`;
- (b) `i` and `j` are identical after NFC and folding;
- (c) `j` = `*` + a literal suffix S, and `i` ends with the literal atoms S (for example `*.c` covers `test*.c`);
- (d) `j` = a literal prefix P + `*`, and `i` starts with the literal atoms P;
- (e) both are fully literal and equal.

Every case is a proof of language inclusion, so there are no false "red" entries. Missed cases only make the
analysis incomplete. A property test checks soundness on random names:
`covered(i,j) ∧ match(i,n) ⇒ match(j,n)`.

#### 2.12.4 Registry loading (`FormatStore::load`, SES-051)

1. Start with the built-ins in `PRESETS` order, with their default `enabled` flags.
2. Read `formats/*.toml`, sorted by file name; `order.toml` is skipped. Each file is parsed with
   `parse_format`. The id must equal the file stem (else warning `user_format_skipped`). The same id as a
   built-in **replaces** the built-in definition (origin `User`), keeping its position. A new id is appended
   to a user list, sorted by id. A file that fails to parse is skipped with `user_format_skipped` (the CLI
   stays usable); `format list --json` shows the skip warnings. The pure merge is
   `FormatRegistry::with_user_formats(self, Vec<(Arc<FileFormat>, PathBuf)>) -> (FormatRegistry, Vec<Warning>)`
   (§1.4.8): it skips, with `user_format_skipped`, a format whose id is not its file stem, whose mask does not
   compile (syntax, or an item with a separator), or whose id another user format already took; one broken
   file never costs the built-ins or the other formats. Compile errors of the rest of a format (patterns,
   ranges) still surface only when that format is selected (§2.12.1).
3. If `formats/order.toml` exists (`schema_version = 1`, `[[format]] id = "…" enabled = true`), the final
   order is the listed ids (unknown ids → warning), followed by unlisted entries in their step-1/2 order.
   Enabled flags come from the list where it gives one.
4. Duplicate ids after merging → `FormatError::DuplicateId` (only possible in a hand-edited `order.toml`;
   reported, not silently resolved).

### 2.13 Importance classification and line keys (TC-012, TC-014, TC-020…TC-022; D-04, D-13, D-14, D-53)

#### 2.13.1 Char classes (precedence top-down)

```
class(char at x in line):
  x inside a match of any compiled `unimportant` pattern on the line → UnimportantPattern
  x inside a grammar token with element e ≠ EE                       → Element(e)
  char is whitespace and x is in the leading-whitespace prefix       → LeadingWhitespace
  char is whitespace and x is in the trailing-whitespace suffix      → TrailingWhitespace
  char is whitespace                                                 → EmbeddedWhitespace
  otherwise                                                          → Other
```

An all-whitespace line counts entirely as leading whitespace. `unimportant` patterns are searched
left-to-right as non-overlapping leftmost-first matches. Literal patterns respect `match_case`.

| Class | important? | case-sensitive? |
|---|---|---|
| UnimportantPattern | no | — |
| Element(e) | `rules.element_important(format, e)`: `importance.elements[name(e)]`, else the format's `ElementInfo::important` | `ElementInfo::case_sensitive` of `e` |
| LeadingWhitespace / EmbeddedWhitespace / TrailingWhitespace | `leading_whitespace` / `embedded_whitespace` / `trailing_whitespace` | yes |
| Other | `everything_else` | `everything_else_case_sensitive` |

#### 2.13.2 Key atoms (general path)

*Superseded in wave 3 (review decisions 2 and 5; the text below the original is binding).* The original encoding
gave every grammar token its own atom with its element id, so a line's key depended on the tokenizer's start state
even when nothing of it was dropped: a line inside a block comment and the same line outside it had different
keys, D-53's equivalence failed for multi-line grammars, and byte-identical lines showed as Changed/Important.
It also left artificial breaks (TC-003) indistinguishable from real ones unless line endings were compared.

```
(original, superseded)
key(line) = for each maximal run of chars with the same (token index, important, case_sensitive): skip if
  !important; grammar token → its TOKEN atom; EE run → the current TEXT atom (merging across removed chars);
  EOL atom when line endings are compared and eol ≠ Artificial
encoding: TEXT = 0x01 varint(len) utf8 · TOKEN = 0x02 u16le(element id) varint(len) utf8 · EOL = 0x03 kind
```

```
(wave 3, binding)
key(line) = bytes, appended to the chunk arena by the tokenize_all callback, built left to right over the chars
            of the line in token order, each char classified per §2.13.1:
  a char of an unimportant class is dropped
  a kept char is appended as UTF-8, folded with case::simple_lower when its class is case-insensitive
  SEP(e1, e2) is appended before a kept char c2 (token t2, element e2) when the previous kept char c1
       (token t1, element e1) is separated from it by dropped chars, t1 ≠ t2, e1 ≠ EE or e2 ≠ EE, and both
       c1 and c2 are word chars (is_word of §2.10.1, with the format's word_chars, D-58)
  at the end: ART if eol == Artificial (always); else EOL(kind) if rules.compare_line_endings
encoding: text = UTF-8 bytes (never ≥ 0xF8) · SEP = 0xFF varint(e1) varint(e2) · EOL = 0xFE kind (1 = LF,
          2 = CRLF, 3 = CR, 4 = None) · ART = 0xFD
trivial(line) ⇔ the key has no text byte ⇔ the key is empty or starts with a marker (≥ 0xF8)
```

Consequences: D-14 holds (`int x` ≠ `intx` under ws-off: a dropped space separates two word chars of different
tokens; `a + b` = `a+b`: no separator next to `+`; in plain text `a b` = `ab`: both chars are EE). A line from
which nothing is dropped or folded is keyed by its raw text plus its end markers, which is its identity key
(§2.13.4), whatever its tokenizer state. A line wrapped by the characters-per-line limit never equals a line ended
by a real terminator. The cancellation token is checked every 64 KiB of a line and before every regex call
(§5.3); the matches of the unimportant patterns are streamed in order (merged on the fly), never stored per match.

The encoding is prefix-free, so two different atom sequences can never produce equal bytes (INV-KEY-1). *Wave 3:*
text bytes are below 0xF8 and every marker starts with a byte ≥ 0xF8 and has a self-delimiting length, so a key
parses uniquely into text and markers (INV-KEY-1). The word-char condition of `SEP` keeps the original D-14
behaviour where removing whitespace cannot merge two tokens into one word (`a + b` = `a+b`, `f (x)` = `f(x)`);
review decision 2 required boundaries *only* where an unimportant char was dropped, and this is a subset of those.

Keys live in per-chunk arenas (`LineBytes`, §1.4.4) and are interned by reference
(`hikasa_diff::intern_pair`, left then right): no per-line allocation and no key copy. The trivial key ids form
the diff `TrivialSet`.

#### 2.13.3 Worked examples (normative test table)

"default" = D-04 (exact apart from line endings). "ws-off" = `--unimportant whitespace`.

| # | Format | Left | Right | Rules | Keys equal | Row |
|---|---|---|---|---|---|---|
| 1 | C | `int  x = 1;` | `int x = 1;` | default | ✗ (identity path: raw text differs) | Changed / Important |
| 2 | C | `int  x = 1;` | `int x = 1;` | ws-off | ✓ | Changed / Unimportant |
| 3 | C | `int x;` | `intx;` | ws-off | ✗ (`[Kw int][Id x]` vs `[Id intx]`; wave 3: `int`·SEP·`x;` vs `intx;`) | Changed / Important |
| 4 | Everything Else | `a b` | `ab` | ws-off | ✓ (TEXT `ab`, D-14) | Changed / Unimportant |
| 5 | C | `x = 1; // old` | `x = 1; // new` | `--unimportant Comment` | ✓ | Changed / Unimportant |
| 6 | C | `x = 1; // old` | `x = 1; // new` | default | ✗ | Changed / Important |
| 7 | SQL | `SELECT a` | `select a` | default (Keyword `case_sensitive = false` in the preset) | ✓ | Changed / Unimportant |
| 8 | Everything Else | `Hello` | `hello` | default | ✗ | Changed / Important |
| 9 | Everything Else | `Hello` | `hello` | `--unimportant case` | ✓ | Changed / Unimportant |
| 10 | any | `x`⏎LF | `x`⏎CRLF | default | ✓, and raw-equal ignoring EOL | Same |
| 11 | any | `x`⏎LF | `x`⏎CRLF | `compare_line_endings = true` | ✗ | Changed / Important |
| 12 | Python | `    return x` | `  return x` | ws-off | ✗ (the `Indent` element stays important) | Changed / Important |
| 13 | Python | `x = 1   ` | `x = 1` | default | ✓ (the `Whitespace` element ships `important = false`) | Changed / Unimportant |
| 14 | C | `   b */` (comment continuation) | `b */` | ws-off | ✓ (leading ws excluded from the token, D-13) | Changed / Unimportant |
| 15 | Everything Else | `x  ` | `x` | default | ✗ | Changed / Important (*wave 3:* normative for every engine with closeness; the scorer gives 999 to texts equal after removing whitespace, §2.15.3, so the pair is Changed, not two orphans) |
| 16 | COBOL | `…` cols 1–72 equal, cols 73–80 `AB000010` vs `AB000020` | | default | ✓ (`Identification` ships unimportant) | Changed / Unimportant |
| 17 | any | `$Id: 12 $` | `$Id: 13 $` | unimportant `\$Id:[^$]*\$` (regex) | ✓ (both trivial) | Changed / Unimportant |
| 18 | any | `` (blank) | (no counterpart) | default | — | LeftOnly / Important (orphans always important) |
| 19 | any | `` (blank) | (no counterpart) | `orphans_always_important = false` | — | LeftOnly / Unimportant (trivial) |

#### 2.13.4 Identity fast path (D-53)

`TextRules::is_identity(format)` holds when: `importance.unimportant` is empty; the three whitespace classes,
`everything_else` and `everything_else_case_sensitive` are true; and for every element id `e ≥ 1` of the format,
`element_important(format, e)` and `ElementInfo::case_sensitive` are true. Then:

```
key(line)     = compare_line_endings ? [eol kind byte] ++ raw line bytes : raw line bytes        (original)
key(line)     = raw line bytes ++ (eol == Artificial ? ART : compare_line_endings ? EOL(kind) : ∅)  (wave 3)
trivial(line) ⇔ the line is empty
no tokenisation runs; start states are computed lazily by TextComparison::prepare_spans
```

Equivalence: when every char is important and case-sensitive, the general key's atom payloads concatenate to
the raw line, and equal raw lines tokenise identically; so two lines have equal general keys iff they have
equal raw text (and equal endings when compared). Both paths induce the same equivalence relation on lines, so
interned ids, alignments, rows, sections and verdicts are identical (property P-TC-7 compares both paths with
`CompareInput::force_tokenize`). Closeness profiles use the important text, which here is the raw line.

*Wave 3:* "equal raw lines tokenise identically" is false in different tokenizer states (a line inside and
outside a block comment), which broke the argument above for multi-line grammars. With the key of §2.13.2 (wave
3) nothing is dropped or folded under identity rules, so the general key **is** the identity key, byte for byte,
in every state: the equivalence holds for every grammar (P-TC-7 runs Everything Else, a single-line grammar,
Rust and XML, and the general path forced by a never-matching unimportant pattern).

### 2.14 Alignment engines (TC-010…TC-012; D-16, D-36, D-43, D-54)

#### 2.14.1 Common rules

- Every algorithm except Unaligned first strips the common prefix and suffix as `Equal` runs.
  INV-ALIGN-6: identical inputs give exactly one `Equal` run (or none, if empty).
- Output canonicalisation: adjacent runs of the same kind are merged. Inside a gap, `Delete` precedes `Insert`.
  Empty runs never appear (INV-ALIGN-4/5).
- A gap is a maximal pair of unmatched ranges `(A, B)`. A gap with both sides non-empty is resolved by §2.15.
- Input limits: either sequence longer than `MAX_SEQUENCE_LEN` → `DiffError::InputTooLarge` before any engine
  runs (hikasa-text already rejects such documents, D-43), so imara-diff's `len < i32::MAX` assertion is
  unreachable (T9).

#### 2.14.2 Unaligned

For `k < min(n, m)`: pair `(k, k)`, `Equal` if the keys are equal, else `Changed` (or `Delete` + `Insert`
under never-align). The rest becomes `Delete` or `Insert`. Skew, weights and closeness are ignored.

#### 2.14.3 Myers and Histogram (imara-diff 0.2)

```
imara_call(algorithm, A, B):                          // every imara-diff call in the crate goes through this
  k = dense.remap(A, B, &mut before, &mut after)       // DenseRemap: local alphabet 0..k (D-16)
  debug_assert!(every token in before/after < k)
  if |A| ≤ 4_096 && |B| ≤ 4_096:                       // D-54 as amended (§1.3.3 item 1)
      cancel.check()?; diff.compute_with(algorithm, &before, &after, k); cancel.check()?
  else:                                                // D-54
      while 2 abandoned helpers still run: wait ≤ 10 ms for one to end; cancel.check()?   // §1.3.3 item 2
      spawn a helper thread that owns `before`, `after` and runs compute_with + postprocess,
      sending the hunks through a std mpsc channel; the caller loops on recv_timeout(10 ms) and checks
      `cancel` between waits → Err(Cancelled) without joining (the helper's result is dropped; the helper
      counts as abandoned until its computation ends)
  diff.postprocess_with(&before, &after, NoSliderHeuristic)   // merges hunks, moves sliders down (deterministic)
  return hunks (local indices = indices into A and B)

Myers / Histogram on a range: hunks → gaps; Equal between hunks; gaps resolved per §2.15
  (Histogram: closeness DP when on; Myers: positional, warning closeness_ignored when closeness is on)
```

DenseRemap matters because imara-diff allocates O(max token id) per call (Myers counts occurrences into a
vector of size max id + 1; Histogram allocates `num_tokens` list heads). Without it, thousands of small calls on
sub-ranges of a 1M-line input would cost O(calls × distinct keys); with it every call costs O(|A| + |B|). The
bench `align_pathological` (§6.4) runs ≥ 10,000 anchor-less gaps.

imara-diff's Myers is a heuristic O(ND) diff, not a guaranteed minimal LCS: its preprocessing prunes tokens that
occur often in the other side when their neighbours are unmatched, and a pruned token never matches (`[a, a]`
against `[b, c, a, d, b]` gives no Equal pair; the LCS is 1; §1.3.3 item 8). Myers and Histogram have no work
budget: on adversarial input (random tokens over a small alphabet) their cost is roughly quadratic (80,000 tokens
per side ≈ 1.7 s), and they stay cancellable through the helper thread. Histogram measured ≤ ≈ 1.6 ns per cell of
`|A|·|B|` for every input shape tried, which is the estimate of Standard's fallback budget (§2.14.5).

#### 2.14.4 Patience

```
patience(range):
  strip prefix/suffix
  U = pairs (i, j) with a[i] = b[j], where the key occurs exactly once in A and exactly once in B and is not trivial
  if U is empty: Myers on the range through imara_call (§2.14.3), its gaps resolved per §2.15
  else: chain = longest increasing subsequence of U by j (patience sorting; j values are unique, so no ties)
        recurse into the gaps between consecutive chain pairs; the chain pairs are Equal
```

#### 2.14.5 Standard (recursive best-anchor, skew tolerance, line weights)

Parameters: skew `S`, weights `w ∈ 0..=4`, `trivial` set, budget `B` (default `64·(n+m) + 1_000_000`
elementary operations), and `WEIGHT_FACTOR = 4`.

```
window(i, j, range) ⇔ |(i − a0) − (j − b0)| ≤ S  ∨  |(a1 − i) − (b1 − j)| ≤ S
                    // displacement relative to the range's start diagonal OR its end diagonal

process(range = (a0..a1, b0..b1)):                 // ranges are processed in document order (explicit stack)
  1. emit Equal(common prefix); shrink; remember common suffix (emitted after the range)
  2. if one side is empty: emit Delete/Insert; emit suffix; return
  3. Stage A (unique anchors, weighted):
       U = {(i, j) | a[i] = b[j], key unique in A and in B, not trivial, window(i, j)}
       score(i, j) = 1 + WEIGHT_FACTOR × max(w_a[i], w_b[j])
       chain = maximum-score chain strictly increasing in i and j (weighted LIS, Fenwick tree over j, O(k log k)).
               Ties: higher total score; then the predecessor with the larger index in U (the nearer one);
               then the chain end with the smallest i.
       blocks = each chain pair extended backward and forward while keys are equal, without crossing the
                previous or next block
  4. if blocks is empty → Stage B (best block):
       for i in A (ascending), non-trivial a[i]: for j ∈ positions_B(a[i]) ∩ window-interval(i):
           skip if (i−1, j−1) is a match of this stage inside the range (non-trivial in Stage B, trivial in
                Stage C), so each maximal block is scored once, from its first line of the stage (§1.3.3 item 4)
           extend forward while equal → block (i, j, len)
           score = Σ over non-trivial lines of the block (1 + WEIGHT_FACTOR × max(w_a, w_b))
       best = max by (score ↓, min(|start disp|, |end disp|) ↑, i ↑, j ↑)
  5. if still empty → Stage C (trivial blocks): as Stage B, but over trivial keys, score = block length
  6. if still empty → the whole range is a gap (§2.15)
  7. otherwise: process(gap before the first block); emit Equal(block); … ; process(gap after the last block)
  8. emit Equal(common suffix)
budget: every examined candidate pair, extension step and hash insertion costs 1. When the budget is exceeded,
        warning work_budget_exhausted is added and every pending range (the current one first, then the others
        in document order) goes to the fallback. Alignment::algorithm() stays Standard.
fallback(range):                                   // D-16 as amended in wave 2 (§1.3.3 item 3)
  emit Equal(common prefix); remember the common suffix
  cells = |A|·|B|                                  // estimated Histogram cost; fallback budget F = 128·(n+m) + 32,000,000
  if cells ≤ F − spent: spent += cells; Histogram through imara_call (§2.14.3, one DenseRemap per range),
        its gaps resolved per §2.15
  else: diff warning alignment_degraded (Alignment::diff_warnings, §1.3.3)
        U = pairs whose key is unique in A and in B and not trivial; U empty → the range is a gap (§2.15)
        chain = LIS of U by j (patience sorting, O(k log k)); each chain pair extended backward and forward
        while equal, without crossing a neighbour → blocks; emit gap, Equal(block), …, gap, where every gap is:
        strip; Histogram if its cells fit F − spent (spent += cells), else resolved per §2.15 (never split again)
  emit Equal(common suffix)
progressive output: when the leftmost pending range is fully resolved, its runs go to the RunSink (ARCH-004).
```

Properties that tests check: INV-ALIGN-1…7, plus these:

- with `S = 0`, `n = m` and closeness off, every Equal pair satisfies `i = j` (§6.2.3);
- a weighted line (priority 4) wins over up to 16 crossing unweighted unique lines;
- given the same budget, results are identical with or without a `RunSink`.

Complexity: one level costs O((n+m)·c), where c is the number of candidates in the window. The work budget
bounds Standard's own work; the fallback budget bounds the Histogram work (≤ ≈ 1.6 ns per cell measured, so
≤ ≈ 0.05 s + 0.2 µs per token); the unique-line split is O(n log n); gap pairing is positional or bounded by the
closeness budgets. Standard is therefore O((n+m) log(n+m)) on every input (test `bounded_standard`: 1,000,000
random lines per side over 50 symbols in < 2 s in release; measured 0.72 s). Histogram, Myers and Patience have
no budget (§2.14.3).

### 2.15 Gap pairing, closeness matching, never-align (TC-011; D-17)

#### 2.15.1 Modes

| Condition | Gap `(A[0..p), B[0..q))` becomes |
|---|---|
| `never_align_differences` | `Delete(A)`, then `Insert(B)` |
| algorithm Standard, Histogram or Patience, and closeness on | the DP pairing below (D-17) |
| algorithm Myers and closeness on | positional, plus warning `closeness_ignored` (once per alignment) |
| otherwise (Unaligned, or closeness off) | positional: pairs `(k, k)` for `k < min(p, q)` (Equal if the keys are equal, else Changed), then the rest as Delete/Insert |

#### 2.15.2 Closeness DP

```
sim(i, j) = 1000 if key(A[i]) == key(B[j])
          = 0    if either line is trivial
          = bigram_similarity(profile(A[i]), profile(B[j]))   // profile over the line's important text (§2.13.2 payloads)
            (wave 3: the scorer of §2.15.3, ≤ 999, under the scorer work budget below)
allowed(i, j) ⇔ sim(i, j) ≥ threshold_permille
M[0][*] = M[*][0] = 0
M[i][j] = max( M[i−1][j], M[i][j−1], allowed(i−1, j−1) ? M[i−1][j−1] + sim(i−1, j−1) : −∞ )
traceback from (p, q): take the diagonal when it attains the max and is allowed; else move up (left line
           unpaired); else move left (right line unpaired)       // deterministic tie-breaking
output in document order: Delete(unpaired left lines before a pair), Insert(unpaired right lines before it),
        then the pair (Equal if keys are equal, else Changed); finally Delete/Insert the remainders
budget: if p·q > max_cells_per_gap, or the cumulative cells would exceed max_cells_total, the gap is paired
        positionally and warning closeness_budget_exhausted is added
scorer budget (wave 3, hikasa-text): every scorer call costs 16 + chars(sample A[i]) + chars(sample B[j]) units,
        every profile built 16 + 4·chars(sample) units, and a call whose length bound (below) cannot reach the
        threshold 4 units (it returns 0: such a pair is never allowed, so the DP result is unchanged); the budget
        of one alignment is 16·(n + m) + 40,000,000 units; once spent, every further call returns 0 (the rest of
        the gap stays unpaired) and warning closeness_budget_exhausted is added (once)
```

*Wave 3 (review decision 3):* the cell budgets bounded the number of scorer calls, not their cost, which grew with
the line length (two unrelated 3.8 MiB files took 44 s, one 100 MiB line used 8 B per char in profiles). The
scorer works on bounded samples and counts its work; counts, never time, decide (T6): hikasa-diff calls the scorer
in a fixed order (one DP at a time, on the calling thread), so the result does not depend on the pool. Profiles
are cached in one window per side (a contiguous run of ≤ 2,048 line indices; the inner side of a DP has at most
2,000 lines). The scorer returns 0 at once after cancellation, and one call costs O(256), so the DP's checks every
4,096 cells stay a few milliseconds apart (G4).

INV-CLOSE-1: the pairs are strictly increasing in both coordinates. INV-CLOSE-2: every pair has
`sim ≥ threshold`. INV-CLOSE-3: the total similarity is maximal, checked by brute force for `p, q ≤ 6`.

Without a scorer (`scorer = None`) every pair of different keys has similarity 0, so with
`threshold_permille ≥ 1` only equal keys pair (§1.3.3 item 6). The DP keeps one direction byte per cell and its
totals along the shorter side of the gap: it fills the table with the longer side in the outer loop (the values
and the traceback are those of the formula above, ties included) and checks cancellation every 4,096 cells.

#### 2.15.3 Bigram similarity

```
profile(s): chars c0..c(n−1); n = 0 → {}; n = 1 → {(c0 << 32) | 0xFFFF_FFFF}; n ≥ 2 → sorted multiset of
            (ck as u64) << 32 | c(k+1) for k in 0..n−1
bigram_similarity(a, b) = (|a| + |b| == 0) ? 1000 : ⌊2000 × |a ∩ b|_multiset / (|a| + |b|)⌋
```

The computation is integer-only, symmetric, and gives 1000 iff the multisets are equal.

*Wave 3 (review decision 1; the hikasa-text scorer, binding):* the definition above leaves one-char texts without a
common bigram with anything, so §2.13.3 row 15 (`x  ` vs `x`) and one-token or brace lines (`}` vs `};`) never
paired. The scorer of a pair of lines with different keys is:

```
sample(line) = the first 256 chars of the line's important text (the key without its markers, §2.13.2)
sim(a, b) = 999                                   if the samples are equal after removing White_Space chars
          = ⌊2000 × |A ∩ B| / (|A| + |B|)⌋        else if either sample has fewer than 3 chars, where A and B are
                                                   the char multisets of the samples (`}` vs `};` = 666)
          = bigram_similarity(profile(sample a), profile(sample b))   otherwise
          capped at 999 (1000 stays reserved for equal keys, which hikasa-diff scores itself)
```

Integer-only, symmetric and deterministic. Lines longer than 256 chars are compared by their first 256 chars of
important text (documented approximation: the score of one call is O(256) whatever the line length). The bigram
intersection is computed with a branch-free merge (same value). A length bound (999 when the samples' whitespace-
free lengths are equal, else ⌊2000·min / (x + y)⌋ over the char counts, or the bigram counts) skips pairs that
cannot reach the threshold.

### 2.16 Inline differences (TC-014)

```
inline_diff(a, b, opts):
  a == b → {}
  chars(a) > max_line_chars or chars(b) > max_line_chars → whole_line = true;
        ranges = the parts left after stripping the common char prefix and suffix
  strip the common char prefix and suffix
  segment each middle part: maximal word run (is_word) | maximal whitespace run | single other char
  intern the segments with a local interner (ids dense by construction); imara Myers over the segment ids;
  postprocess (no heuristic)
  for each hunk: if both sides are non-empty and their total chars ≤ refine_max_chars:
                     intern the hunk's chars locally (dense ids, never raw code points); char-level imara Myers
                     → differing char ranges
                 else: the hunk's segment byte ranges
  per side: merge adjacent ranges
```

The classification in hikasa-text (`TextComparison::inline`) works per row:

0. *Wave 3 (review decision 6):* on a `Changed`/`Unimportant` row every range is unimportant: the keys are equal,
   so no important char differs (a case change next to an ignored space, `Hello World` vs `helloworld` under
   `--unimportant whitespace,case`, is blue, never red). Steps 1–3 apply to Important rows.
1. For each pair of corresponding differing ranges (left part ↔ right part of the same hunk), if
   `fold(left) == fold(right)` and every char involved has a case-insensitive class, the pair is
   **unimportant**. This is a case-only change inside a case-insensitive element.
2. Otherwise the ranges are split by importance class (`split_ranges_by_class`, with class = the char's
   importance from §2.13.1).
3. Finally, ranges are widened to grapheme-cluster boundaries (`unicode-segmentation`), so that a combining
   mark is never split.

*Wave 3 (bounded on long lines, review decision 8):* when either line is longer than 4 × `max_line_chars` bytes
(so certainly over the char limit), the whole-line ranges are computed by a block-wise byte scan of the common
prefix and suffix (backed off to char boundaries; the result equals `inline_diff`'s). The classification uses the
spans of `TextComparison::spans`, which cover the first 256 KiB of a line (`SPAN_LIMIT_BYTES`; the rest is one
span of class `Other`, important), steps 1 and 2 walk the spans (never one call per byte), step 1 tests the
classes before folding the texts, and grapheme boundaries are found locally at each range end
(`GraphemeCursor`). One call is O(line length) memory copies at most.

INV-INLINE-1: ranges are sorted, disjoint, non-empty and on char boundaries. INV-INLINE-2: removing the ranges
leaves an equal "common" text on both sides. INV-INLINE-3: if `a == b` there are no ranges. INV-INLINE-4: the
output is deterministic.

### 2.17 Rows, sections, statistics, verdict

#### 2.17.1 Rows

Rows are expanded from the runs, in order:

- **Equal pair `(i, j)`**: the row is `Same` if `line(i) == line(j)`, plus `eol(i) == eol(j)` when
  `compare_line_endings` is set (EOL is ignored otherwise). Else the row is `Changed`/`Unimportant`: the keys
  are equal, so only unimportant characters differ.
- **Changed pair**: `Changed`/`Important`.
- *Wave 3:* "raw-equal" means equal text, both or neither ended by an artificial break (TC-003), and equal endings
  when `compare_line_endings` is set. A **Changed pair whose lines are raw-equal** is `Changed`/`Unimportant`
  (review decision 2: byte-identical lines never show as important; their keys can differ only through the
  tokenizer state, e.g. a case-insensitive number inside and outside a block comment). The identity path needs no
  check (its keys are the raw lines plus end markers).
- **Delete `i`**: `LeftOnly`, Important if `orphans_always_important || !trivial(i)`, else Unimportant.
  **Insert `j`**: `RightOnly`, by the same rule.

INV-ROW-1: `Same` ⇔ importance `None`. INV-TC-2: the left line indices of the rows, in order, are exactly
`0..left_lines`, and likewise on the right (coverage, order).

#### 2.17.2 Sections

A section is a maximal run of non-`Same` rows. If the effective format has `lines_independent`, every
non-`Same` row is its own section instead. Sections are numbered from 0 in row order. Each section has:

- `kind` = `Add` (only RightOnly rows), `Delete` (only LeftOnly rows), or `Change`;
- `importance` = Important if any of its rows is Important, else Unimportant;
- `left_lines`/`right_lines` = the covered line ranges. An empty range starts at the insertion point: the next
  line index on that side.

#### 2.17.3 Statistics (D-18)

For each section, `class` = its importance (I or U). `left` and `right` count its rows that have a line on
that side.

```
for each section s, with class = s.importance (I or U):
   L = #rows of s with a left line;  R = #rows of s with a right line
   Add    → class.added   += R
   Delete → class.deleted += L
   Change → class.original += L; class.changed += R
rows, same_rows, changed_rows, left_only_rows, right_only_rows, important_rows, unimportant_rows: direct counts
```

#### 2.17.4 Statistics CSV (RPT "Statistics")

The header is
`SourceFile,CompareFile,Total,IAdded,IDeleted,IChanged,IOriginal,UAdded,UDeleted,UChanged,UOriginal,Errors`.
There is one data row per pair. `Total` = rows. `Errors` = the error message, or empty. Formatting follows
RFC 4180: CRLF line ends, and fields containing `,` `"` CR or LF are quoted with doubled `"`.

#### 2.17.5 Verdict (shared by `text compare`, `quick --rules`, `RulesComparator`, and GUI views)

```
verdict(iu):
  bytes_identical                               → BinarySame
  left or right document is binary (§2.9)       → BinaryDifferent           (D-12: binary content is never "rules-same")
  no rows with importance Important or Unimportant → RulesSame
  any Important row                             → ImportantDifferences
  else                                          → iu ? RulesSame : UnimportantOnly
```

### 2.18 Text display filters and context (TC-040, TC-035, RPT-010)

```
cat(row, iu) = Same          if state == Same || (iu && importance == Unimportant)
             = LeftOrphans   if state == LeftOnly
             = RightOrphans  if state == RightOnly
             = Differences   otherwise (Changed)
visible(row) = match filter:
   All → true; None → false; Same → cat == Same; Differences → cat ≠ Same
   Context{N} → ∃ d with cat(d) ≠ Same and |row − d| ≤ N          (two O(rows) distance sweeps)
   Toggles{bits} → bits ∋ cat
visible'(row) = visible(row) || row ∈ ⋃ opts.expanded                       // expanded gap bars (TC-035)
FilteredView: maximal runs of visible' rows are kept as rows; each maximal run of hidden rows becomes one Gap item
```

INV-VIEW-1: the view items cover `0..row_count` in order, without overlap. INV-VIEW-2: no two Gap items are
adjacent. INV-VIEW-3: `Context{N}` with hunks at most `2N` apart merges them, which matches unified-diff hunk
merging. The patch renderers use exactly this view, so the "Context" display and `--report patch` agree.

### 2.19 Rules-based quick compare (FC-025, TC-016; D-12, D-15, D-25)

```
quick_compare_bytes(L, R, format, rules, load):
  0. (wave 3) format conversion is External and no runner is given → Error (the Convert/unsupported error of
     load_document without a runner)
  1. L == R                                    → BinarySame
  1b. (wave 3) External conversion of both sides with the runner, as load_document's byte stage; steps 2–6 see the
     converted bytes (the documents are marked converted)
  2. sniff(L) or sniff(R) is binary            → BinaryDifferent (D-12)
  3. build both documents (TextDocument::from_bytes) and compute keys (§2.13) on the same code path as compare()
     (identity fast path included); a document limit hit (D-43) → Error{too_large}
  4. key sequences equal → every line pair raw-equal (EOL per rules) ? RulesSame : UnimportantOnly
  5. rules.importance.orphans_always_important → ImportantDifferences        (shortcut, no alignment: linear time)
     (wave 3: only when the line counts differ or both sides' keys are context-free — every line's key equals the
     key it gets from the Normal tokenizer state, checked by re-tokenising the lines that start in another state)
  6. otherwise run the identical alignment and row classification as compare() and return verdict(false)

RulesComparator::compare(l, r, ctl):
  a. either length known and > max_bytes       → BinaryComparator semantics (streaming, §2.20), result
                                                  BinarySame/BinaryDifferent with warning rules_size_limit (D-25)
  b. bytes = ContentSource::read_all(max_bytes, ctl.cancel) per side (ReadError::Cancelled → Err(Cancelled))
  c. format = forced, else detect(l.name()) when non-empty, else detect(r.name()), else Everything Else (D-15);
     a non-Text/Table kind → byte comparison (D-11): BinarySame/BinaryDifferent
  d. quick_compare_bytes(l, r, format, rules, load, ctl)
```

**Consistency theorem (TC-016).** For any inputs and rules,
`quick_compare_bytes(...) == compare(...).verdict(false)`.

1. Step 1 equals verdict rule 1, and step 2 equals verdict rule 2.
2. In step 4, every algorithm maps identical key sequences to a single Equal run (INV-ALIGN-6). So the rows are
   Same or Changed/Unimportant according to raw equality, which is exactly verdict rules 3 and 5.
3. Step 5: when the key sequences differ, every alignment has at least one non-Equal run (INV-ALIGN-1/2). Such a
   run yields a Changed row (always Important) or an orphan row, which is Important because
   `orphans_always_important` is on. So `compare` has an Important row and returns ImportantDifferences.
   *Wave 3:* a Changed row is Important unless its lines are raw-equal (§2.17.1). With differing line counts some
   row is an orphan. With equal counts and context-free keys, raw-equal lines have equal keys, so no Changed pair
   is raw-equal. Otherwise step 5 is skipped and step 6 decides.
4. Step 6 runs the same functions.

A property test checks the theorem for every algorithm and both values of `orphans_always_important`
(P-TC-2, §6.2.5). `RulesComparator` adds only the documented size limit and the D-11/D-15 format rules, which
`hikasa quick --rules` applies identically.

*Wave 3 (review decisions 7 and 8):* the theorem needs both paths to use the same regex limits for the unimportant
patterns and the same External conversion runner. `quick_compare_bytes_with(.., &QuickOptions { limits, external },
..)` and `compare_with_limits(.., limits, ..)` take them; `RulesComparator` compiles with `registry.limits()` and
takes the runner with `with_external(temp_dir, runner)`; a session passes the same registry limits and runner to
`load_document`/`compare_with_limits` for `text compare` (tests `regex_limits_agree_between_rules_comparator_and_compare`,
`external_conversions_agree_on_every_path`). In `RulesComparator` step c, the right side is detected only for the
`format_mismatch` warning: a right-side detection error (a user format that does not compile) no longer fails the
pair; the left format is used and the warning names the error.

### 2.20 Binary compare and CRC (FC-025, VFS-020, ARCH-081)

```
compare_sources(a, b, opts):
  1. both local and same_file::is_same_file(pa, pb)       → Same          (same object)
  2. both lengths known, different, and both > 4096 bytes  → Different{first_difference: None}
     (a reported size ≤ StreamOptions::UNRELIABLE_SIZE_MAX = 4096 never decides alone: procfs reports 0 and
     sysfs 4096 for files of other lengths; such files are read, which is cheap)
  3. policy Auto, both local, length ≥ min_len:
        mmap::compare_files_mapped: 8 MiB chunks claimed in increasing order by opts.threads threads (the
        caller plus scoped helpers, §5.1); each chunk is mapped, pre-faulted (Linux MADV_POPULATE_READ),
        compared and unmapped by the thread that claimed it; cancel is checked before every claim; the lowest
        differing offset wins (no chunk below it is skipped), so the outcome does not depend on the thread
        count; a chunk below the first difference that cannot be mapped or pre-faulted abandons the attempt
        (None); afterwards both files are re-stat'ed: an unchanged (length, mtime) pair keeps the result, a
        change discards it (None)
        → result unless None
  4. streaming: fill buf_a fully, fill buf_b with the same length (cancel checked before every buffer and
     between the partial reads that fill it); compare slices; the first unequal index gives the offset; EOF on
     one side first → Different{first_difference: offset}
crc32_source(s): known_crc32 → mmap (policy first, then local_path; the chunk CRCs of opts.threads threads are
                 combined in offset order with crc32fast::Hasher::combine, which is exact; same abandon and
                 re-stat rules) → streaming crc32fast::Hasher
CrcComparator:    lengths known, different and both > 4096 → BinaryDifferent;
                  else crc(a) == crc(b) ? BinarySame : BinaryDifferent
BinaryComparator: compare_sources → Same ? BinarySame : BinaryDifferent
```

Errors become `ContentResult::Error`, and cancellation becomes `Err(Cancelled)`. Every finished CRC or comparison
ends with one unconditional completion report (`Ctl::progress_now`, `done == total` = the bytes processed).
INV-VFS-2: the mmap path (for every thread count), the streaming path and the naive whole-buffer comparison give
the same outcome, including `first_difference`, for all inputs and buffer sizes. A property test with random
contents, buffer sizes from 4 KiB to 1 MiB and 1–4 threads checks this on whole files; a unit property checks
files of up to 256 mapped chunks of 1 byte to 64 KiB with 1–8 threads.

### 2.21 Exit-code mapping (CLI-004; D-01, D-02, D-03, D-22, D-23, D-35)

| Command | Outcome | Code |
|---|---|---|
| `quick`, `text compare` | BinarySame / RulesSame / UnimportantOnly / UnimportantOnly with `--ignore-unimportant` / ImportantDifferences / BinaryDifferent | 1 / 2 / 12 / 2 / 13 / 11 |
| `quick --size` | sizes equal / different | 1 / 11 |
| `quick --crc`, `quick --binary` | BinarySame / BinaryDifferent | 1 / 11 |
| `quick`, `text compare` | any `ApiError` (spec, load, unsupported scheme, too large, format, rules, cancelled) | 100 |
| `folder compare` | verdict NoDifferences / UnimportantOnly / Differences / Incomplete | 2 / 12 / 13 / 100 |
| `folder compare` | `FolderError::BaseNotLoadable`, `BaseNotFolder`, or `LoadFailed` for a base | 107 |
| `folder compare` | any other `ApiError` | 100 |
| `format …`, `vfs …`, `completions`, `version` | success / any error | 0 / 100 |
| `gui open`, `--gui` (no `--wait`) | opened / no GUI with `--no-launch` / no executable / launched GUI unreachable (retry budget) / other | 0 / 100 / 103 / 102 / 100 |
| `gui open --wait`, `--gui --wait` | view closed / connection lost while waiting / no GUI with `--no-launch` / launched GUI unreachable / no executable / other | the view's code (D-23) / 102 / 102 / 102 / 103 / 100 |
| `gui ping`, `list`, `close`, `quit` | success / not running / other | 0 / 100 / 100 |
| any | clap usage error / `--help` / `--version` | 100 / 0 / 0 |
| any | panic (caught) / invariant violation | 100 |
| any | broken pipe on stdout | the computed code |

`ExitCode::for_content(result, iu)` (and `for_kind`) implements the "verdict" rows. `ExitCode::for_verdict(v)`
implements the folder row. `ApiError::exit_code(op)` implements the error rows: with `op = GuiOpen { wait }`,
`IpcError::NotRunning` maps to 102 when `wait` and to 100 otherwise; `IpcError::LaunchTimeout` and
`IpcError::ConnectionLost` map to 102; `ApiError::GuiNotFound` maps to 103; with `op = GuiControl`, `NotRunning`
maps to 100. Each function is a total `match` (no `_` arm over its own crate's enums, §0.4 rule 9), and the whole
table is a unit test.

*Amended in the wave-4 session review (§1.8.7):* `IpcError::InsecureEndpoint` (a socket folder another user
controls) is 100 for every `gui` command, and nothing is launched; a view whose waiter the GUI dropped closes with
reason `abandoned` and code 100, a view still open when the GUI's server shuts down with reason `shutdown` and
code 100 (the view's real code is unknown); a protocol error the GUI sends with id 0 is 100 (not 102); a folder
view closed while still scanning reports `Incomplete` → 100 (`ViewOutcome::close`).

### 2.22 Configuration and cache directories (SES-050)

```
resolve_config_dir(facts, explicit, exists):
  1. explicit = Some(d) → absolute(d, facts.cwd) (`RelativeConfigDir` if relative without cwd) → Explicit
  2. facts.exe_dir = Some(x) && exists(x / "hikasa-portable.toml") → x → Portable
  3. platform default (default_config_dir) → User:
       Linux/OtherUnix: (facts.xdg_config_home if absolute, else facts.home/.config) / "hikasa"
       MacOs:           facts.home / "Library/Application Support/Hikasa"
       Windows:         facts.appdata / "Hikasa"
     a missing ingredient → NoConfigDir
  cache_dir:  Explicit/Portable → dir / "cache"
              User → Linux/OtherUnix: (facts.xdg_cache_home if absolute, else facts.home/.cache) / "hikasa";
                     MacOs: facts.home / "Library/Caches/Hikasa"; Windows: facts.local_appdata / "Hikasa" / "cache"
              (a missing ingredient → facts.temp_dir / ("hikasa-cache-" + id), id as in §2.23)
  is_default: dir == default_config_dir(facts)
```

*Amended in the wave-4 session review (§1.8.7 items 7, 30):* `--config-dir ""` is `EmptyConfigDir`; the temp
fallback of the cache folder is per user (`hikasa-cache-<id>`); the stdin spool verifies the cache folder (owned by
the user, not writable by others; the temp fallback must be a real folder) and creates `stdin/` and `<n>/` with
mode 0700 and the file with mode 0600, all with create-new semantics.

Read-only commands never create either folder. Files that are missing mean defaults.

### 2.23 IPC endpoint (ARCH-090, ARCH-091; D-47)

```
Endpoint::for_config(facts, config):
  0. facts.ipc_endpoint_override = Some(o) → Unix: socket path o, lock o + ".lock";
                                             Windows: pipe name o, lock facts.temp_dir \ (o + ".lock")
  1. suffix = config.is_default ? "" : "-" + hex8(fnv1a64(config.dir bytes))     // first 8 lowercase hex digits
     (config.dir bytes: the OsStr bytes on Unix, UTF-16LE code units on Windows)
  2. Linux / OtherUnix:
       dir  = facts.xdg_runtime_dir when absolute, else facts.temp_dir / ("hikasa-" + id)   // Hikasa's own folder
              where id = facts.uid, else sanitize(facts.user), else "user"
       path = dir / ("hikasa" + suffix + ".sock")
     MacOs: dir = facts.temp_dir (the per-user $TMPDIR); path as above
     if len(path) > 100 bytes: path = dir / ("hk-" + hex16(fnv1a64(path)) + ".sock")
       and if still > 100 bytes: path = "/tmp/hikasa-" + id + "/hk-" + hex16(…) + ".sock" (Hikasa's own folder)
     lock = path + ".lock"
  3. Windows: name = "hikasa-" + sanitize(facts.user or "user") + suffix; address = \\.\pipe\<name>;
     lock = facts.temp_dir \ (name + ".lock")
  sanitize(s): every char outside [A-Za-z0-9_.-] becomes '_'

IpcServer::bind(endpoint) on Unix:
  - Hikasa's own folder: create with mode 0700 when missing (DirBuilderExt::mode); verify mode & 0o077 == 0
  - InstanceLock::try_acquire(lock) → AlreadyRunning when held
  - remove a stale socket file (the lock proves no live server owns it), bind, chmod 0600
  - verify the folder owner equals the socket file's owner (our euid) → else InsecureEndpoint
  - per connection: peer_creds().euid() must equal the socket file's owner uid, else the connection is closed

Folder verification (Endpoint::verify_folder; amended in the wave-4 session review, §1.8.7 item 11), Unix only:
  dir = the folder of the socket path; uid = endpoint.uid (EnvFacts::uid; unknown → refuse)
  - dir missing → NotRunning
  - dir is a symbolic link: Hikasa's own folder → refuse; else the link must belong to uid (then follow it)
  - dir must be a folder owned by uid with mode & 0o022 == 0 (Hikasa's own folder: mode & 0o077 == 0)
  - else InsecureEndpoint (exit 100)
  The client verifies before every connect (also before and between launch retries) and never contacts, launches
  into or waits on a refused folder. The server creates Hikasa's own folder with mode 0700 (create-new), refuses one
  it does not own, verifies the folder before binding, removes a stale *socket* only (any other file at the socket
  path is refused, never deleted), and closes connections whose peer credentials cannot be read.
```

| # | Situation | Endpoint |
|---|---|---|
| 1 | Linux, default config, `XDG_RUNTIME_DIR=/run/user/1000` | `/run/user/1000/hikasa.sock`, lock `/run/user/1000/hikasa.sock.lock` |
| 2 | Linux, `--config-dir /data/hk` | `/run/user/1000/hikasa-<h8>.sock` (`h8` from `/data/hk`) |
| 3 | Linux, portable install, no `XDG_RUNTIME_DIR`, uid 1000 | `<temp>/hikasa-1000/hikasa-<h8>.sock` (folder 0700, owner verified) |
| 4 | macOS default | `$TMPDIR/hikasa.sock` |
| 5 | Windows, user `Anna Lee`, default | `\\.\pipe\hikasa-Anna_Lee`, lock `%TEMP%\hikasa-Anna_Lee.lock` |
| 6 | Unix path longer than 100 bytes | `/tmp/hikasa-<id>/hk-<h16>.sock` (Hikasa's own folder; corrected in the wave-4 review: the `<dir>/hk-<h16>.sock` step is always longer than 100 bytes as well, §1.8.7 item 3) |
| 7 | `HIKASA_IPC_ENDPOINT=/tmp/t/x.sock` | `/tmp/t/x.sock` (tests) |

Two GUIs with different configuration folders therefore never share a socket or a lock, while every launch with
the same configuration reaches the same single instance (ARCH-090).

---

## 3. Cross-crate data flows

Arrows read "calls". Every flow starts in an adapter, either the CLI or the GUI, and goes through exactly one
`Service` method (ARCH-013).

### 3.1 Quick compare: `hikasa quick a b [--rules]`

```
cli main ── capture_env, Cli::try_parse, read stdin only if a spec is "-" (Blob{bytes, mtime: now, name: --stdin-name}),
            Service::new(ServiceInit{…})
  └─ cmd::quick::run ── QuickRequest{left, right, method, ignore_unimportant, format, rules_file, rules_patch}
       └─ Service::quick (inside run_guarded)
            1. parse_spec(left/right)                      core::Spec::parse_os (SpecContext from EnvFacts)
            2. vfs.stat(both) → Err → ApiError::LoadFailed{side}                    (exit 100, D-01)
               a folder on either side → ApiError::InvalidArgument("quick compares files")
            3. method:
               Size   → sizes equal ? BinarySame : BinaryDifferent                  (no I/O beyond stat)
               Crc    → vfs::CrcComparator.compare(VfsSource×2, ctl)
               Binary → vfs::BinaryComparator.compare(VfsSource×2, ctl)
               Rules  → rules = TextRules::default() ← --rules-file ← rules_patch; rules.validate
                        format = forced (--format) or D-15 (left name, else right name; "-" uses the blob's
                                 name hint), via registry.detect
                        bytes: ContentSource::read_all(4 GiB − 1, cancel) per side
                        non-Text/Table kind → byte comparison (D-11)
                        text::quick_compare_bytes(left, right, format, rules, load, ctl)   (§2.19, incl. shortcut)
            4. exit = ExitCode::for_content(result, ignore_unimportant)              (§2.21)
  └─ output: human → nothing on stdout; --json → JsonEnvelope<QuickDto> ("hikasa.quick.v1")
  └─ process::exit(exit.code())
```

### 3.2 Text compare: `hikasa text compare a b --report side-by-side --html --output r.html`

```
Service::text_compare(req, job, observer)
  1. specs ← parse_spec; names for D-15 (Spec::file_name, or the stdin blob's name hint)
     format = forced, or registry.detect(first named side); the other named side's detection only informs
             (warning format_mismatch when it differs)
  2. documents are loaded with rayon::join on the service pool:
       text::load_document(vfs, spec, format, load_opts(side), external, ctl) per side
       load_opts(side).decode.override_encoding ← req.encodings.<side> (TC-001 per-pane override)
       load_opts(side).decode.locale_fallback ← EnvFacts.locale_encoding (UTF-8 default)
       errors → ApiError::LoadFailed{side} / Text(TooLarge | TooManyLines) (exit 100, D-01)
     observer.loaded(&left, &right)
  3. bytes_identical = (raw_left == raw_right); the raw buffers are dropped
  4. rules = default ← --rules-file ← patch (incl. --unimportant items and patterns); rules.validate(limits)
  5. cmp = text::compare(CompareInput{…}, Some(&pool), job.ctl(), observer)
        keys: rules.is_identity(format) ? raw lines into per-chunk arenas (no tokenisation, D-53)
                                        : formats::tokenize_all (parallel speculative chunks) writing atom keys
              + weights when the algorithm resolves to Standard (D-36)
        → diff::intern_pair (borrowed arena slices) → diff::align(AlignInput, AlignOptions{algorithm: Auto resolved
          per D-36, closeness per D-17, …}, scorer = bigram profiles, sink = observer adapter)
        → rows (§2.17.1) → sections → stats → warnings
  6. verdict = cmp.verdict(ignore_unimportant); exit = ExitCode::for_content(verdict, iu)
CLI:
  --gui    → Service::gui_open(GuiOpenRequest{kind: TextCompare, specs, options from the flags}) instead (D-50)
  --json   → report::text_compare_json(&cmp, &ViewOptions{filter from --display/--context (default mismatches)})
  else     → Service::render_text(resp, TextReportOptions{layout, display, iu, …}, buffer)
             → report::render_text_report(&cmp, opts, ReportMeta::for_text(…, Some(clock.now()), …), buffer)
  --output → Service::write_output(target, buffer) (VfsRouter → LocalVfs::open_write → AtomicWrite::commit)
```

### 3.3 Folder compare with the injected rules-based comparator

```
Service::folder_prepare(req)                                          (no I/O on the bases)
  1. specs ← parse_spec
  2. rules = FolderRules::default() ← --rules-file ← Criteria::parse_list(--criteria).patch ← req.patch
     (incl. --filters → NameFilterLists; DateFilter/SizeFilter already parsed by the CLI through core)
  3. text_rules (only when content method = Rules) = TextRules::default() ← --text-rules-file ← text_patch
  4. model = folder::FolderModel::new(left, right, rules)  → PreparedFolderCompare
Service::folder_run(prepared, job, observer)
  5. engines = ContentEngines {
         crc:    Arc::new(vfs::CrcComparator    { opts: StreamOptions{buffer_size: options.binary_buffer_size, …} }),
         binary: Arc::new(vfs::BinaryComparator { opts: … }),
         rules:  if method == Rules {
                     Some(Arc::new(text::RulesComparator::new(registry.clone(), text_rules, load_opts)?
                                   .with_max_bytes(DEFAULT_RULES_MAX_BYTES)))   // hikasa-text object, typed as core trait
                 } else { None } }
  6. folder::compare_into(&model, &FolderEnv{ vfs: &router, content: &engines, pool: Some(&pool),
         ctl: job.ctl(), observer, now: &clock.now(), content_budget: options.content_memory_budget_mib << 20 })
       a. stat both bases (errors → FolderError::BaseNotLoadable / BaseNotFolder → exit 107); resolve CaseMode::Auto
          from capabilities; set the bases in the model → ScanEvent::Started
       b. NameFilter::from_lists(…, case) per side; OtherFilters resolved with `now`
       c. pool.scope, one task per folder pair: list the left and right folders in parallel with
          ListOptions{follow, prefilter: name-mask exclusion (D-45)} → align_names (§2.5) → children in canonical
          order → stat the filtered side of FC-054 pairs → quick_tests (§2.6.1)
          → write lock: append the children as one contiguous block, set states, update counters and totals on
            every ancestor (§2.6.4); unlock; generation += 1 → FolderListed{folder}
          → content tests as rayon tasks: budget.acquire(min(size_l + size_r, budget)) →
               engines.<method>.compare(&VfsSource(left), &VfsSource(right), ctl)
               (the RulesComparator reads both sources via ContentSource::read_all ≤ 128 MiB, then runs
                text::quick_compare_bytes; folder never sees a hikasa-text type)
            → write lock: state (§2.6.2), flags, counters on the ancestors; unlock → NodesUpdated (batched ≤ 256)
          → recurse into subfolder pairs (and into orphan subfolders when scan_orphan_folders)
       d. finalize: mark finished, check_invariants (debug builds), Finished{complete}
  7. verdict = cmp.verdict(iu) (O(1) from the root counters); exit = ExitCode::for_verdict(verdict)
CLI: --gui → Service::gui_open (D-50); --report list|side-by-side|summary|json, --html, --output (as in 3.2)
```

The dependency direction is kept: `hikasa-folder` → `hikasa-core::ContentComparator`, and `hikasa-text`
implements it. Only `hikasa-session` knows both crates (ARCH-010/011).

### 3.4 Reports

```
render_text_report(cmp, opts, meta, out):
  header (RPT-004): title; "Generated: <meta.generated_at RFC 3339 with offset> by <generator>" (omitted if None);
                    "Left: <spec> (<encoding>, <eol>)", "Right: …"; "Mode: <meta.mode>"; "Rules: <meta.filters>";
                    per-side errors (meta.errors) and per-side warnings (the comparison's and the rendering's);
                    in patches every header line starts with "# "
  layout (patches as amended in wave 4, §1.7.3 item 1):
    identical(row) = state Same, equal terminators, same written BOM (byte-for-byte equal lines);
    groups = maximal runs of rows that are not identical; hunks = groups clustered when ≤ 2n identical rows apart,
        widened by n identical rows each side (clamped); iu drops the clusters without an important row
    AnnotatedUnified{n} / Patch(Unified{n}) → one "@@" hunk per cluster;
        header "@@ -l,s +l,s @@" (",1" omitted when s = 1; when s = 0, l = the line before),
        annotated: suffix " important" or " unimportant"; identical rows ' ' (from the left file), each group
        as its '-' lines then its '+' lines; text + original terminator (CRLF kept; lone CR and artificial
        break → LF with warning patch_lossy_eol); U+FEFF before line 1 of a UTF-8 file with a BOM;
        "\ No newline at end of file" after a line whose ending is None (always the last line of its side);
        patch_encoding for a non-UTF-8 or lossily decoded side when the files differ (§1.7.3 item 2)
    Patch(Context{n}) → "***************", "*** a,b ****" / "--- c,d ----" (empty range after line l: "l+1,l",
        at the start "0"), markers "! " (group with lines on both sides) "- " "+ " "  " (identical rows, from the
        left file); a part without group lines is written as its range line only
    Patch(Normal)     → per group (n = 0; per differing row for lines_independent formats):
        "<l>a<r1>,<r2>" | "<l1>,<l2>d<r>" | "<l1>,<l2>c<r1>,<r2>", "< " / "---" / "> "
    every patch, applied to the left file, reproduces the right file byte for byte (INV-RPT-2)
    SideBySide (text) → columns: line no | left text | marker | line no | right text; marker ' ' same, '|'
        changed, '~' unimportant change, '<' left only, '>' right only; tabs expanded to the format's tab size;
        widths via unicode-width (string width of character clusters); control and bidirectional formatting
        characters escaped; source line numbers (physical_line); wrap per Wrap; hidden runs →
        "      … N lines hidden …"
    SideBySide (HTML) → one self-contained document; <style> embedded; classes .imp/.unimp/.lo/.ro/.gap;
        inline ranges as <span class="imp|unimp">; every text node HTML-escaped; legend table; no scripts,
        no external resources; no ids derived from memory addresses or time
    Summary           → the counts of TextStats + verdict (text or HTML)
    Statistics        → §2.17.4 CSV
render_folder_report(cmp, opts, meta, out):   (collects the visible rows under one cmp.read() guard, then formats
                                               them reading the model in chunks of 4096 rows; §1.7.3)
    List       → "<marker> <path>[/]" per visible row (§4.3.3; names escaped, "\" as "\\"), then "# <summary>"
                 unless empty
    SideBySide → header; per visible row: indent(depth) name | columns… | marker | name | columns…
                 (folder Size = the subtree total of that side, FC-040; blank columns for name-only and
                 stat-failed entries, a blank Size for a folder whose listing failed on that side)
    Summary    → FolderStats per category (13-reports: same, left/right newer, left/right orphan, different, …)
    Json       → serde_json::to_writer_pretty(json::FolderReportJson::new(&folder_compare_json(cmp, &opts.display)))
                 ("schema" first, then the result fields), followed by "\n"
ReportMeta::for_text / for_folder build `mode` and `filters` with fixed English wording from the options and
rules (deterministic; no locale).
```

Determinism (INV-RPT-1): byte-identical output for identical `(cmp, opts, meta)`. Exactness (INV-RPT-2): every
patch reproduces the right file from the left one; what it cannot carry is warned (`patch_lossy_eol`,
`patch_encoding`), and with Ignore Unimportant the dropped hunks are by design. RPT-011 is satisfied
because the GUI's "Report…" command calls the same functions through `Service::render_*`.

### 3.5 CLI command → API, and rules precedence

| Command | Service method | Engine entry points |
|---|---|---|
| `quick` | `quick` | `vfs::{CrcComparator, BinaryComparator}`, `text::quick_compare_bytes` |
| `text compare` | `text_compare`, `render_text` / `report::text_compare_json`, `write_output`; `gui_open` with `--gui` | `text::load_document`, `text::compare`, `report::render_text_report` |
| `folder compare` | `folder_compare` (= `folder_prepare` + `folder_run`), `render_folder` / `report::folder_compare_json`, `write_output`; `gui_open` with `--gui` | `folder::compare_into`, `report::render_folder_report` |
| `format list/show/detect/tokens` | `format_*` | `FormatStore::load`, `FormatRegistry::{entries, find, detect, shadow_report}`, `formats::tokenize_all` |
| `vfs ls/stat/cat` | `vfs_*` | `VfsRouter::{list, stat, open_read}` |
| `gui …` | `gui_*` | `api::gui::{normalize_legacy_args, parse_open_args}`, `ipc::{connect_or_launch, IpcClient}` |
| `completions` | (CLI-local: `clap_complete::generate`) | — |
| `version` | (CLI-local DTO) | — |

Rules precedence, from lowest to highest: built-in defaults (`Default`), then (M2: the session type's
Defaults), then the `--rules-file` TOML, then `--criteria` (folder compare), then individual flags
(`*RulesPatch`; *wave-4 review:* the unix flags are separate patch fields, so each flag overrides one test, §1.8.7
item 19). The text compare and folder compare tables in §1.9.2 also accept `--rules <file>` as a spelling
of `--rules-file` (docs/12). `quick` uses `--rules` as the method flag (CLI-003), so there the file option is only
`--rules-file`.

### 3.6 IPC: `hikasa gui open a.c b.c --wait` (CLI-G01, CLI-010, ARCH-092)

```
CLI (hikasa)                               session::ipc                           GUI (hikasa-gui)
────────────                               ────────────                           ────────────────
normalize_legacy_args + parse_open_args (D-51) → OpenArgs
Service::gui_open(req, Some(&launcher))
  specs → canonical absolute strings (parse_spec with the CLI's cwd); "-" → spooled file in <cache>/stdin/
          (D-49); clipboard:// → InvalidArgument (100)
  kind  ← --fv or infer_view_kind (2 files → text_compare)
  endpoint ← Endpoint::for_config(facts, config)                                  (§2.23)
  connect_or_launch(endpoint, …) ──── connect: NotRunning
  launcher.launch(&service.gui_launch()) ── spawn hikasa-gui [--config-dir D] ──► starts; Endpoint::for_config
                                                                                   IpcServer::bind (lock, 0700/0600)
                                                                                   + spawn(handler)
  retry connect: launcher.sleep(50 ms … 500 ms), budget 10 s of sleeps ─────────► accept (peer euid == owner)
  ► {"id":1,"type":"hello","client":"hikasa 0.1.0","version":1,"min_version":1}
                                       ◄ {"id":1,"type":"hello","server":"hikasa-gui 0.1.0","version":1}
  ► {"id":2,"type":"open_view","kind":"text_compare","specs":["/w/a.c","/w/b.c"],"options":{…},"wait":true}
                                                    handler.handle(OpenView, Some(waiter)) → opens the view
                                       ◄ {"id":2,"type":"view_opened","view":7}
  IpcClient::wait_view_closed(2)  … user compares, edits, closes the view …
                                                    waiter.view_closed(7, ViewResult::new(…),
                                                                       ExitCode::for_kind(verdict, iu))
                                       ◄ {"id":2,"type":"view_closed","view":7,"result":{…},"exit_code":12}
  return GuiOpenOutcome::Closed{…, exit_code: 12}; delete the spooled stdin file, if any
process::exit(12)
```

Failure paths (D-23):

- No GUI and `--no-launch`: 102 with `--wait`, 100 without.
- No executable found among `gui_exe_candidates` → `ApiError::GuiNotFound` → 103.
- Still unreachable after the retry budget → `IpcError::LaunchTimeout` → 102.
- EOF before `view_closed` → `ConnectionLost` → 102.
- An immediate response missing for longer than the request timeout (10 s) → `IpcError::Timeout` → 100.
- Version mismatch → `Error{incompatible_version}` → 100.
- A second GUI launch (`hikasa-gui x y`) cannot acquire the instance lock. It forwards its request with
  `Service::forward_open` (absolute specs, no wait) and exits with 0 (ARCH-090).
- *Amended in the wave-4 session review (§1.8.7):* the launch carries `--config-dir` for explicit and portable
  configurations and `HIKASA_IPC_ENDPOINT` when overridden (the launched GUI listens where the CLI waits); before
  connecting the client verifies the socket folder (insecure → 100, no launch); the request is validated locally
  (`check_open`) before the GUI is contacted; the whole immediate-response wait is one 10 s deadline; a GUI that
  drops its waiter answers `ViewClosed{reason: abandoned, exit_code: 100}` and a GUI shutting down closes its
  views with `shutdown` (100), so `--wait` never hangs on a live GUI; a GUI process that exits still gives
  `ConnectionLost` → 102.

### 3.7 GUI flows enabled by the contract (wave 6)

- **Text, first screen.** `Service::spawn(TextCompare, …)` runs `text_compare` with an observer that forwards
  `loaded` (render unaligned panes at once), `phase` and `partial` (confirmed prefixes, progressively for
  Standard) into a channel; the final `Arc<TextComparison>` replaces the partial state. Visible rows read
  `row(i)`, `spans` and `inline(row)` with a small LRU cache.
- **Folder, incremental.** `folder_prepare` returns the live model at once; `Service::spawn` runs `folder_run`
  with a `ChannelObserver`; every frame the UI compares `model.generation()` with the last rendered value and,
  when it changed, takes `model.read()` and rebuilds the visible rows (O(visible)).
- **Stop.** `JobHandle::cancel()`; text compare returns `Err(Cancelled)` (the GUI keeps what it showed); folder
  compare returns an incomplete comparison whose counters are consistent.
- **Views from IPC** (*wave-4 review*). `Service::open_view_request` turns an `OpenView` into the view's request
  and initial display options; closing a waited view goes through `ViewOutcome::close` (D-23); the GUI's own
  command line goes through `resolve_open`, a second instance through `forward_open`.

---

## 4. Stable JSON shapes and the IPC protocol

### 4.1 Serialisation rules (normative for every `--json` document and every golden file)

1. Exactly one JSON document per invocation on stdout: UTF-8, pretty-printed with 2-space indentation,
   followed by `\n`. This holds on error too (`"ok": false`).
2. Keys are `snake_case`. Field order = DTO declaration order (`serde` preserves it). Enum values are
   `snake_case` strings. Maps are emitted with sorted keys (`BTreeMap`).
3. Absent optional values are `null` and the key is **always present**. Arrays are never `null`.
4. Numbers are integers only. Sizes are bytes (`u64`). CRC32 is an 8-digit lowercase hex string. Similarity
   values are per mille.
5. Timestamps are RFC 3339 UTC (`FileTime`), e.g. `"2026-10-19T13:12:00.25Z"`.
6. Specs are canonical strings (§2.1). Paths inside a comparison are `RelPath` strings with `/`.
7. Line numbers are **1-based**. Row indices and byte offsets are 0-based. Ranges are half-open two-element
   arrays `[start, end)`. Line ranges use 1-based starts: `[11, 14]` means lines 11–13.
8. File text appears as JSON strings (lossy-decoded text contains U+FFFD; the document reports `malformed`).
   A non-UTF-8 file name appears lossily with `"name_lossy": true`.
9. Bit sets (aggregates, diff flags, capabilities) are arrays of `snake_case` names in canonical bit order.
10. Each document carries `"schema": "hikasa.<command>.v<major>"`. Adding fields keeps the major version; renaming
   or removing fields bumps it. Consumers must ignore unknown fields.

### 4.2 Envelope

```rust
/// Every `--json` document (owned by hikasa_session::api::dto).
#[derive(Debug, Serialize)]
pub struct JsonEnvelope<T: Serialize> {
    pub schema: &'static str,
    pub hikasa_version: &'static str,
    pub ok: bool,
    pub exit_code: u8,
    pub result: Option<T>,
    pub warnings: Vec<Warning>,
    pub error: Option<ErrorReport>,
}
```

*Amended in the wave-4 session review (§1.8.7 items 8, 27):* `warnings` start with the startup warnings of the
service (unknown `options.toml` sections, skipped user formats), followed by the command's own; `exit_code` of
`gui open --wait` is the raw view code (`JsonEnvelope::success_raw`); a usage error with `--json` is
`hikasa.usage_error.v1` (`result: null`, `error.kind: "invalid_input"`, exit 100).

```json
{
  "schema": "hikasa.text_compare.v1",
  "hikasa_version": "0.1.0",
  "ok": false,
  "exit_code": 100,
  "result": null,
  "warnings": [],
  "error": {
    "kind": "not_found",
    "message": "left input /w/missing.c: /w/missing.c: not found",
    "spec": "/w/missing.c",
    "causes": ["/w/missing.c: not found"]
  }
}
```

### 4.3 Result shapes per command

#### 4.3.1 `hikasa.quick.v1`

```json
{ "left": "/a/x.c", "right": "/b/x.c", "method": "rules", "result": { "result": "unimportant_only" },
  "ignore_unimportant": false,
  "format": { "id": "c-cpp", "name": "C, C++, C#, ObjC Source", "kind": "text", "reason": { "reason": "mask", "id": "c-cpp", "mask": "*.c" } } }
```

`format` is `null` for the `size`, `crc` and `binary` methods. `result.result` ∈ `binary_same`, `rules_same`,
`unimportant_only`, `important_differences`, `binary_different`, `error`. For `error` there is also an `error`
object (an `ErrorReport`).

#### 4.3.2 `hikasa.text_compare.v1`

```json
{
  "left":  { "spec": "/a/x.c", "size": 3456, "mtime": "2026-10-19T13:12:00Z", "lines": 120, "encoding": "UTF-8",
             "encoding_source": "utf8_valid", "bom": false, "eol": "lf", "binary": false, "malformed": 0, "converted": false },
  "right": { "spec": "/b/x.c", "size": 3501, "mtime": "2026-10-19T14:00:00Z", "lines": 123, "encoding": "UTF-8",
             "encoding_source": "utf8_valid", "bom": false, "eol": "mixed", "binary": false, "malformed": 0, "converted": false },
  "format": { "id": "c-cpp", "name": "C, C++, C#, ObjC Source", "kind": "text", "reason": { "reason": "mask", "id": "c-cpp", "mask": "*.c" } },
  "detection": { "left": { "reason": "mask", "id": "c-cpp", "mask": "*.c" }, "right": { "reason": "mask", "id": "c-cpp", "mask": "*.c" } },
  "algorithm": "histogram",
  "rules": { "schema_version": 1, "importance": { "…": "TextRules as TOML-equivalent JSON" }, "alignment": { "…": "…" } },
  "ignore_unimportant": false,
  "verdict": { "result": "important_differences" },
  "stats": { "rows": 125, "same_rows": 110, "changed_rows": 8, "left_only_rows": 2, "right_only_rows": 5,
             "important_rows": 12, "unimportant_rows": 3,
             "important": { "added": 5, "deleted": 2, "changed": 6, "original": 6 },
             "unimportant": { "added": 0, "deleted": 0, "changed": 2, "original": 2 },
             "sections": 4, "important_sections": 3, "left_lines": 120, "right_lines": 123 },
  "sections": [
    { "id": 0, "kind": "change", "importance": "important", "rows": [10, 14], "left_lines": [11, 14], "right_lines": [11, 15] }
  ],
  "display": { "mode": "mismatches", "lines": null, "toggles": ["left_orphans", "right_orphans", "differences"] },
  "items": [
    { "type": "gap", "rows": [0, 10] },
    { "type": "row", "row": 10, "state": "changed", "importance": "important",
      "left":  { "line": 11, "text": "int x = 1;" },
      "right": { "line": 11, "text": "int x = 2;" },
      "inline": { "left": [ { "start": 8, "end": 9, "important": true } ],
                  "right": [ { "start": 8, "end": 9, "important": true } ], "whole_line": false } },
    { "type": "row", "row": 13, "state": "right_only", "importance": "important",
      "left": null, "right": { "line": 14, "text": "return y;" }, "inline": null }
  ]
}
```

`display` defaults to `mismatches` for JSON, unless `--display` or `--context` is given. Human output defaults to
the annotated unified diff with `context_lines` (default 3). `display.lines` is set for `context` only (else
`null`); `display.toggles` is always an array: the shown categories (§1.7.3 item 4). `line` numbers and section line
ranges are logical (1-based indices of the document's lines); they equal the source line numbers unless a
characters-per-line limit broke lines (the human side-by-side views show source lines, §1.7.3 item 6).

#### 4.3.3 `hikasa.folder_compare.v1` (D-29)

```json
{
  "left":  { "spec": "/a", "capabilities": ["read", "write", "list", "seek", "mtime", "unix_mode", "unix_owner", "symlinks", "local_path", "case_sensitive"] },
  "right": { "spec": "/b", "capabilities": ["…"] },
  "rules": { "schema_version": 1, "criteria": { "…": "…" }, "alignment": { "…": "…" }, "filters": { "…": "…" }, "handling": { "…": "…" } },
  "case": "sensitive",
  "complete": true,
  "display": { "filter": "differences", "folders": "compare_structure", "ignore_unimportant": false, "suppress_filters": false,
               "toggles": ["left_orphans", "right_orphans", "left_newer", "right_newer", "differences"] },
  "verdict": "differences",
  "stats": { "files":   { "same": 120, "left_newer": 0, "right_newer": 1, "different": 1, "left_orphan": 1, "right_orphan": 1, "unknown": 0, "error": 0, "filtered": 3 },
             "folders": { "same": 10, "left_newer": 0, "right_newer": 0, "different": 2, "left_orphan": 0, "right_orphan": 0, "unknown": 0, "error": 0, "filtered": 0 },
             "left":  { "files": 123, "folders": 12, "bytes": 1048576 },
             "right": { "files": 123, "folders": 12, "bytes": 1049000 },
             "content": { "binary_same": 0, "rules_same": 0, "unimportant_only": 0, "important_differences": 1, "binary_different": 0, "error": 0, "not_compared": 0 } },
  "items": [
    { "path": "src", "depth": 0, "kind": "folder", "state": "different", "category": "different", "scan": "scanned",
      "aggregate": ["same", "right_newer", "different"], "diffs": [], "flags": [], "time": null, "content": null,
      "errors": [],
      "left":  { "name": "src", "name_lossy": false, "size": 524288, "mtime": "2026-10-19T10:00:00Z", "ctime": null,
                 "attrs": null, "mode": 493, "permissions": "drwxr-xr-x", "uid": 1000, "gid": 1000, "link_target": null,
                 "followed": false, "crc32": null, "filtered": false, "stat_skipped": false, "dir": null },
      "right": { "name": "src", "…": "…" } },
    { "path": "src/main.rs", "depth": 1, "kind": "file", "state": "right_newer", "category": "right_newer",
      "scan": "not_applicable", "aggregate": null, "diffs": ["size", "time", "content"], "flags": [],
      "time": "right_newer", "content": { "method": "rules", "result": { "result": "important_differences" }, "warnings": [] },
      "errors": [], "left": { "name": "main.rs", "…": "…" }, "right": { "name": "main.rs", "…": "…" } },
    { "path": "docs/old.md", "depth": 1, "kind": "file", "state": "left_orphan", "category": "left_orphan",
      "scan": "not_applicable", "aggregate": null, "diffs": [], "flags": [], "time": null, "content": null,
      "errors": [], "left": { "name": "old.md", "…": "…" }, "right": null }
  ]
}
```

The human `List` layout (the default for `folder compare`) prints one line per visible row. Markers:

| Marker | Meaning |
|---|---|
| `=` | same |
| `>` | left newer |
| `<` | right newer |
| `!` | different |
| `~` | different, with only unimportant content differences |
| `-` | left only |
| `+` | right only |
| `?` | unknown |
| `E` | error |
| `.` | filtered (only with `--suppress-filters`) |

Item fields: `category` is computed with the document's `display.ignore_unimportant`; `aggregate` (folders only)
lists the categories whose descendant count (files and folders, same `iu`) is non-zero, in `Category` order;
`flags` are `NodeFlags` names; `errors` are `{ "side": "left"|"right"|null, "error": ErrorReport }`. In a side
object, a folder's `size` is the total size of the files below it on that side (FC-040), `stat_skipped` marks a
name-only entry (D-45), and `dir` is the containing folder in flatten mode (else `null`). Node ids are never
printed. `display.toggles` (additive) lists the categories the filter shows. The JSON folder *report*
(`--report json`, `FolderLayout::Json`) is this document with `"schema": "hikasa.folder_compare.v1"` as its first
key (§4.1 rule 10); under `--json` the envelope carries the schema and `result` is the document above (§1.7.3
item 4).

Folders end with `/`. Control and bidirectional formatting characters in names are escaped (`\n`, `\t`, `\u{…}`),
and a backslash is written `\\`, so every printed path names exactly one item (§1.7.3 item 5). The last line
counts files: `# <n> same, <n> left newer, <n> right newer, <n> different, <n> left only, <n> right only, <n>
unknown, <n> errors`; it is left out when the comparison holds no item. *Amended in wave 4:* `--quiet` keeps it
(CLI invariant 3: `--quiet` never changes stdout).

```
!  src/
<  src/main.rs
-  docs/old.md
+  docs/new.md
# 120 same, 0 left newer, 1 right newer, 1 different, 1 left only, 1 right only, 0 unknown, 0 errors
```

#### 4.3.4 `format`, `vfs`, `gui`, `version`

| Schema | `result` shape |
|---|---|
| `hikasa.format_list.v1` | `{ "config_dir": str, "formats": [ { "position": u32 (1-based), "id", "name", "kind", "mask", "enabled": bool, "origin": "built_in"\|"user", "path": str\|null, "status": {"status": "visible"\|"shadowed"\|"manual_only"\|"disabled", "by": [ids]?} } ] }` |
| `hikasa.format_show.v1` | `{ "id", "origin", "path", "enabled", "format": <FileFormat as JSON>, "compile_error": ErrorReport\|null }` |
| `hikasa.format_detect.v1` | `{ "file": str, "format": { "id": str\|null, "name", "kind" }, "reason": DetectReason }` |
| `hikasa.format_tokens.v1` | `{ "file", "format": {…}, "encoding": {…}, "regex_errors": u64, "lines": [ { "line": u32, "state_in": LineState, "tokens": [ { "start", "end", "element": str, "item": u16\|null, "text": str } ] } ] }` |
| `hikasa.vfs_list.v1` | `{ "spec", "entries": [VfsEntry sorted by name bytes], "errors": [ { "name", "error": ErrorReport } ] }` |
| `hikasa.vfs_stat.v1` | `{ "spec", "entry": VfsEntry, "capabilities": [str] }` |
| `hikasa.vfs_cat.v1` | `{ "spec", "size": u64, "data_base64": str }` (D-32) |
| `hikasa.gui_open.v1` | `{ "view": u64, "waited": bool, "result": ViewResult\|null, "view_exit_code": u8\|null, "spooled_stdin": str\|null }` |
| `hikasa.gui_ping.v1` | `{ "endpoint": str, "server": str, "protocol_version": u32 }` |
| `hikasa.gui_list.v1` | `{ "views": [ { "view", "kind", "title", "specs": [str] } ] }` |
| `hikasa.gui_close.v1` / `hikasa.gui_quit.v1` | `{ "view": u64 }` / `{}` |
| `hikasa.version.v1` | `{ "version": "0.1.0", "ipc_protocol": 1, "schemas": [str] }` |
| `hikasa.completions.v1` | `{ "shell": str, "script": str }` (wave-4 review) |
| `hikasa.usage_error.v1` | `null` (the envelope's `error` describes the usage error; wave-4 review) |

`VfsEntry` = `{ "name", "name_lossy", "kind", "size", "mtime", "ctime", "attrs": "AHR"|null, "mode": u32|null,
"permissions": str|null, "uid": u32|null, "gid": u32|null, "link_target": str|null, "followed": bool,
"crc32": str|null }`.

### 4.4 IPC protocol v1 (`hikasa_session::ipc::protocol`, ARCH-091)

#### 4.4.1 Framing and endpoint

Each frame is a `u32` little-endian payload length `N` (1 ≤ N ≤ 16 MiB), followed by `N` bytes of UTF-8 JSON.
A zero or oversized length, or invalid UTF-8/JSON, is a protocol error: the server replies `Error{protocol}`
with `id = 0` and closes the connection. EOF at a frame boundary is a clean close. EOF inside a frame is
`ConnectionLost`. The endpoint (socket path or pipe name, plus the instance lock) is computed per user and
configuration folder (§2.23, D-47).

#### 4.4.2 Messages

```rust
/// A framed message: request id + flattened body. The id is chosen by the client and strictly increases per
/// connection; responses (and ViewClosed) echo it. Serialised in declaration order: `id`, then `type`, then the
/// body's fields (the `#[serde(flatten)]` struct form). Decoded in two phases (§4.4.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub id: u64,
    #[serde(flatten)]
    pub body: T,
}

/// Client → server. Unknown `type` values decode to `Unknown` (D-46).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello { client: String, version: u32, min_version: u32 },
    OpenView { kind: ViewKind, specs: Vec<String>, #[serde(default)] options: OpenOptions, #[serde(default)] wait: bool },
    LoadSession { name: String, #[serde(default)] wait: bool },     // M1: Error{unsupported_request}
    LoadWorkspace { name: String },                                  // M1: Error{unsupported_request}
    RunScript { path: String, #[serde(default)] args: Vec<String> }, // M1: Error{unsupported_request}
    ListViews,
    CloseView { view: ViewId },
    Ping,
    Shutdown,
    /// Any `type` this build does not know. Never sent.
    #[serde(other)]
    Unknown,
}

/// Server → client. Unknown `type` values decode to `Unknown`, which clients treat as a protocol error for the
/// pending request (D-46).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello { server: String, version: u32 },
    ViewOpened { view: ViewId },
    ViewClosed { view: ViewId, result: ViewResult, exit_code: u8 },
    Views { views: Vec<ViewInfo> },
    Ok,
    Pong,
    /// `code` is an `IpcErrorCode` name; clients accept codes they do not know.
    Error { code: String, message: String },
    /// Any `type` this build does not know. Never sent.
    #[serde(other)]
    Unknown,
}

/// View/session types (00-overview §2). M1 GUI serves `folder_compare` and `text_compare`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    FolderCompare, FolderMerge, FolderSync, TextCompare, TextMerge, TextEdit, TextPatch,
    TableCompare, HexCompare, PictureCompare, MediaCompare, VersionCompare, RegistryCompare,
    /// A kind added by a newer peer; the server answers `Error{unsupported_view}`.
    #[serde(other)]
    Unknown,
}

impl ViewKind {
    /// Parses `--fv` values: "Text Compare", "text-compare", "text_compare", … (ASCII-case-insensitive).
    pub fn parse(s: &str) -> Option<ViewKind>;
}

/// View id (unique per GUI process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ViewId(pub u64);

/// Options of `OpenView` (CLI-003 subset for M1, plus the `--gui` rules of D-50). Every field defaults when
/// absent; unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenOptions {
    /// Pane titles: left, right, center, output (`--title1..4`).
    pub titles: [Option<String>; 4],
    /// Read-only switches (`--ro`, `--ro1`, `--ro2`).
    pub read_only: ReadOnly,
    /// Name filter for the initial folder comparison (`--filters`).
    pub filters: Option<String>,
    /// `--expandall`.
    pub expand_all: bool,
    /// Forced format id or name.
    pub format: Option<String>,
    /// `--ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// Absolute path of a text rules file (`--rules-file` with `--gui`).
    pub text_rules_file: Option<String>,
    /// Text rule overrides (`--gui`).
    pub text_rules: Option<TextRulesPatch>,
    /// Absolute path of a folder rules file.
    pub folder_rules_file: Option<String>,
    /// `--criteria` text.
    pub criteria: Option<String>,
    /// Folder rule overrides (`--gui`).
    pub folder_rules: Option<FolderRulesPatch>,
    /// Recognised CLI-003 switches that M1 does not implement, forwarded verbatim (name → value, "" for flags).
    pub extra: BTreeMap<String, String>,
    // Added in the wave-4 session review (D-50, §1.8.7 item 20); absent → defaults, ignored by older peers:
    /// Per-side encoding overrides of a text view (`--encoding`, `--left-encoding`, `--right-encoding`).
    pub encodings: [Option<String>; 2],
    /// `--display` word (text or folder vocabulary; parsed by the GUI with the core parsers).
    pub display: Option<String>,
    /// `--context <n>` (text views).
    pub context_lines: Option<u32>,
    /// `--folders` word (folder views).
    pub folders: Option<String>,
    /// `--suppress-filters` (folder views).
    pub suppress_filters: bool,
}

/// Read-only switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadOnly { pub all: bool, pub left: bool, pub right: bool }

/// Why a view closed (sent as its snake_case name inside `ViewResult::reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason { User, Shutdown, LoadFailed, Abandoned }   // Abandoned: wave-4 review, §1.8.7 item 12

/// Result reported with `ViewClosed`. Plain strings, so that values added later never break an older peer
/// (D-46); `ViewClosed::exit_code` is authoritative for the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewResult {
    /// File views: `ContentKind` name at close time ("unimportant_only", …).
    pub verdict: Option<String>,
    /// Folder views: `CompareVerdict` name at close time.
    pub folder_verdict: Option<String>,
    /// Whether edits were saved.
    pub saved: bool,
    /// `CloseReason` name ("user", "shutdown", "load_failed", "abandoned"; unknown values allowed).
    pub reason: String,
}

impl ViewResult {
    /// Builds the strings from typed values.
    pub fn new(verdict: Option<ContentKind>, folder_verdict: Option<CompareVerdict>, saved: bool, reason: CloseReason) -> ViewResult;
    /// `ContentKind::parse(verdict)`.
    pub fn verdict_kind(&self) -> Option<ContentKind>;
}

/// `ListViews` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewInfo { pub view: ViewId, pub kind: ViewKind, pub title: String, pub specs: Vec<String> }

/// Server identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo { pub server: String, pub version: u32 }

/// Error codes produced by this build (sent as strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpcErrorCode { Protocol, IncompatibleVersion, UnsupportedRequest, UnsupportedView, InvalidRequest, NotFound, Internal }

impl IpcErrorCode {
    /// snake_case wire name ("unsupported_request", …).
    pub const fn as_str(self) -> &'static str;
}
```

#### 4.4.3 Protocol rules

1. The first client message is `Hello`. The server chooses `v = min(PROTOCOL_VERSION, client.version)`. If
   `v < client.min_version`, it replies `Error{incompatible_version}` and closes. Otherwise it replies
   `Hello{server, version: v}`. Any other first message gets `Error{protocol}` and a close.
2. Every request gets exactly one immediate response with the same `id`. `OpenView{wait: true}` additionally
   gets exactly one later `ViewClosed` with the same `id`, unless the connection is gone.
3. Two-phase decode (D-46): the frame is parsed as a `serde_json::Value`; `id` is read from it (missing or not a
   `u64` → `Error{protocol}` with `id = 0` and close); then the rest is decoded into `Request`. A known `type`
   whose fields do not decode → `Error{invalid_request}` with that id. An unknown `type` → `Request::Unknown` →
   `Error{unsupported_request}` with that id. Unknown fields are ignored. Clients decode responses the same way.
4. `specs` are canonical, absolute spec strings. The server never resolves relative paths, because the GUI has a
   different current directory. `-` and `clipboard://` never appear (D-49).
5. M1 server support: `Hello`, `OpenView` (`folder_compare`, `text_compare`; other kinds, including
   `ViewKind::Unknown`, get `Error{unsupported_view}`), `ListViews`, `CloseView` (unknown id → `Error{not_found}`),
   `Ping` → `Pong`, `Shutdown` → `Ok`, after which views close with reason `shutdown` and waiters are notified.
6. Security on Unix: the socket sits in a per-user folder (`$XDG_RUNTIME_DIR`, macOS `$TMPDIR`, or Hikasa's own
   0700 folder with a verified owner), the socket file has mode 0600, and the server checks
   `peer_creds().euid() == owner uid of the socket file`. Windows: a per-user pipe name. Hardening the DACL is a
   P1 item (Appendix C).
7. Timeouts are client-side: 10 s for an immediate response (`IpcClient::connect` timeout), none while waiting
   for `ViewClosed`.
8. *Wave-4 session review (§1.8.7 items 11–15):* rule 2 is enforced by the server library (a dropped waiter sends
   `ViewClosed{reason: "abandoned", exit_code: 100}`; `ServerHandle::shutdown` sends `reason: "shutdown"`,
   exit 100, to every outstanding waiter and closes every connection); the 10 s of rule 7 are one deadline per
   call, and a client keeps at most 16 messages for other requests (more is a protocol error); a server message
   with id 0 fails the pending request; a protocol error closes the connection also when a waiter holds it; a
   frame with more than 262,144 JSON values is a protocol error (bounded decoder memory); server writes go through
   a bounded per-connection queue with a 5 s send timeout, and a client that does not read is dropped; rule 6 adds
   the client-side folder verification of §2.23.

Wire examples (one frame per line):

```json
{"id":1,"type":"hello","client":"hikasa 0.1.0","version":1,"min_version":1}
{"id":1,"type":"hello","server":"hikasa-gui 0.1.0","version":1}
{"id":2,"type":"open_view","kind":"text_compare","specs":["/w/a.c","/w/b.c"],"options":{"titles":[null,null,null,null],"read_only":{"all":false,"left":false,"right":false},"filters":null,"expand_all":false,"format":null,"ignore_unimportant":false,"text_rules_file":null,"text_rules":null,"folder_rules_file":null,"criteria":null,"folder_rules":null,"extra":{},"encodings":[null,null],"display":null,"context_lines":null,"folders":null,"suppress_filters":false},"wait":true}
{"id":2,"type":"view_opened","view":7}
{"id":2,"type":"view_closed","view":7,"result":{"verdict":"unimportant_only","folder_verdict":null,"saved":false,"reason":"user"},"exit_code":12}
{"id":3,"type":"ping"}
{"id":3,"type":"pong"}
{"id":4,"type":"open_session_v9","name":"x"}
{"id":4,"type":"error","code":"unsupported_request","message":"unknown request type `open_session_v9`"}
```

---

## 5. Concurrency, cancellation, progress, memory and error handling

### 5.1 Threads and pools

| Component | Threads | Rule |
|---|---|---|
| Engines (diff, formats, text, folder) | none of their own, except the imara helper thread (D-54) | Synchronous APIs (ARCH-080). Parallel work runs **only** on the `Option<&rayon::ThreadPool>` passed in, through `pool.install` / `pool.scope` / `pool.join`. `None` means the current thread. The rayon global pool is never touched (T7). |
| imara helper (`hikasa-diff`) | 1 short-lived `std::thread` per imara call on > 4,096 tokens per side; at most 2 abandoned helpers per process (D-54, §1.3.3) | Owns copies of the remapped tokens; the caller polls `cancel` every 10 ms (`recv_timeout`) and abandons the helper on cancellation (D-54). The helper of a cancelled call runs until its call ends; while 2 such helpers run, a new helper call waits for one of them, polling `cancel` every 10 ms (`Condvar::wait_timeout`). |
| VFS mapped helpers (`hikasa-vfs`) | up to `StreamOptions::threads - 1` short-lived `std::thread::scope` threads per mapped CRC or comparison (`crc32_source`, `compare_sources`, the comparators); none by default (`threads = 1`) | Bounded (≤ 15) and scoped: started and joined inside one call, no pool, no global state (T7). They claim 8 MiB chunks from a shared counter and check `cancel` before every claim, so a cancel waits for at most one chunk per thread, as on one thread (≤ 100 ms on a warm cache; on a cold slow device the chunk's read time sets the latency, as before); results never depend on the count (T2, §2.20). A thread the OS refuses is skipped. `hikasa-session` passes `min(4, its pool size)`; a content task on the service pool may therefore run up to 3 helpers (G3: one thread cannot reach the page-cache read rate, §6.4). |
| `hikasa-session::Service` | one `rayon::ThreadPool` (`available_parallelism`, thread names `hikasa-worker-N`) | Owned by the service and shared by every job. |
| Jobs (`Service::spawn`, GUI) | one `std::thread` per job (`hikasa-job-<id>`) | The job body runs on its job thread and uses the pool through the engines it calls (*amended in the wave-4 session review*: it no longer enters the pool with `install`, so a long job does not pin a worker; §1.8.7 item 10); panics are caught (§5.6). |
| Folder scanner | tasks on the service pool | Both sides are listed with `join`. Each child folder pair is a task, and each content test is a task that first acquires bytes from the `ByteBudget` (D-25). |
| Text compare | tasks on the service pool | The two sides are loaded with `join`, keys are built in chunks (§2.10.6), interning is sequential (deterministic ids), alignment is sequential. |
| IPC server (GUI) | one accept thread, plus one reader and one writer thread per connection | Handlers must not block: they post to the GUI event loop and return. Writes go through the connection's bounded queue (64 frames, 5 s send timeout; a stuck client is dropped), so `view_closed` never blocks. `ServerHandle::shutdown` wakes the accept thread by self-connecting, notifies the outstanding waiters and closes the connections (§1.8.7 items 12, 13). |
| IPC client (CLI, second GUI instance) | one reader thread per client (`hikasa-ipc-read`) | Decodes frames into a bounded channel; calls wait with a `crossbeam_channel::after` deadline (no clock read, T1); dropping the client closes the connection (§1.8.7 item 14). |
| CLI | the main thread waits for the service call | Progress is rendered from pool threads through `StderrProgress` (an internal mutex). |

Enforcement: each parallel crate has a "global-pool trap" integration test. The test binary builds the rayon
global pool with a `start_handler` that panics, then runs the public APIs with explicit pools. Any implicit use
of the global pool makes the test fail.

### 5.2 Shared state and locks

| State | Primitive | Held during | Never held during |
|---|---|---|---|
| `FolderModel` store | `parking_lot::RwLock` | write: inserting one directory batch, or applying one content result (counters O(depth)); read: one `TreeRead` (one GUI frame or one 4096-row report chunk) | I/O, comparator calls, budget waits, event delivery |
| `ByteBudget` | `parking_lot::Mutex` + `Condvar` | the arithmetic inside `acquire`/drop of the guard | anything else; a content task drops its `BudgetGuard` before taking the model write lock |
| `FormatRegistry` compiled formats | immutable `Arc` + `OnceLock` per entry | the first compilation of one format | — |
| `MemoryVfs` | `std::sync::RwLock` | lookups and inserts | reads of the shared `Arc<[u8]>` bytes |
| IPC connection writers / waiters | a bounded queue per connection, drained by its writer thread; a `parking_lot::Mutex` per waiter gate | the writer: writing one frame (5 s send timeout); a gate: a state change | waiting for the GUI; no caller of `view_closed` ever waits for a write (wave-4 review) |
| Job progress | bounded `crossbeam_channel` (64) | `try_send` (drops when full) | — |

Lock order: the model lock and the budget mutex are never held together. Immutable results (`TextComparison`,
`TextDocument`, `CompiledFormat`, `FolderComparison` after the scan) need no locks for correctness (the finished
model's lock is uncontended).

### 5.3 Cancellation

- `CancelToken::check()` runs at these check points: every 1024 directory entries listed; every stream buffer
  and every partial read that fills it, and every 8 MiB mapped chunk (on every mapped helper thread), in binary
  compare and CRC; every 1 MiB and every partial read in `ContentSource::read_all`, and every 1 MiB in document
  loading; every 8192-line chunk in tokenisation, and inside a line every 64 KiB of scanned text and before
  every regex engine call (one line may be megabytes long); every 65,536 alignment steps; before and after each inline
  imara call, and every 10 ms while a helper thread runs a large one; every closeness gap; every 4096 rows built;
  every 10 ms while waiting for content-test budget; every 20 ms while waiting for an external conversion.
- Latency target: ≤ 100 ms from `cancel()` to return (gate G4, §6.4). There is no exception: large imara calls
  run on the helper thread (D-54).
- Semantics:
  - text compare, quick compare and reports return `Err(Cancelled)`; the GUI keeps what it already showed;
  - folder compare after the bases are loaded returns `Ok` with `is_complete() == false`, unscanned folders
    `Unknown` (counters consistent), and verdict `Incomplete`;
  - a content test returns `Err(Cancelled)`, and its node stays `Unknown`;
  - a cancelled external conversion kills the child and removes temporary files.
- The CLI installs no signal handler (§0.7). SIGINT terminates the process. `--output` files are written
  atomically, so no partial report can appear.

### 5.4 Progress

| Phase (`core::Phase`) | Unit | Emitted by |
|---|---|---|
| reading | bytes | `text::load_document`, `vfs::*` streaming |
| decoding | bytes | `TextDocument::from_bytes` |
| tokenizing | lines | hikasa-text around `formats::tokenize_all` (from its per-line callback, which receives every line number; `tokenize_all` has no progress parameter, §1.4.4) and the identity-path key build |
| aligning | items (resolved ranges) | hikasa-text, from the `RunSink` calls of `diff::align`, which has no progress sink (§1.3.3 item 9): Standard and Unaligned progressively; Myers and Histogram the common prefix, then the rest after the imara-diff call (indeterminate meanwhile) |
| classifying | lines | text rows/sections |
| scanning | folders (`current` = relative path) | `folder::compare_into` |
| comparing_content | bytes per pair + items overall | folder content tests |
| hashing | bytes | CRC |
| reporting | items | report renderers (large reports only) |

Engines report through `Ctl::progress(phase, || Progress { … })`, which builds the record only when the sink's
throttle is open (`ProgressSink::due`), so hot loops allocate no `current` string between ticks. The service
wraps the adapter's sink in `ThrottledSink` (≤ 20 Hz per phase). Phase changes and completion (`done == total`)
always pass the throttle. Progress never affects results (T1). The CLI shows progress only when stderr is a TTY
and `--quiet` is not set. The GUI receives progress through the lossy `JobHandle::progress` channel.

### 5.5 Error model

```
crate errors (thiserror)            ─Classify→ ErrorKind ─┐
  SpecError, MaskError, ReadError, VfsError,              ├→ ErrorReport (JSON "error", IPC, node errors)
  DiffError, FormatError, PatternError,                    │
  ConvertError, TextError, FolderError,                    │
  ReportError, SessionError, IpcError                      │
        └──────────── From ───────────→ ApiError ──exit_code(ApiOp)→ ExitCode (§2.21)
```

1. **Whole-operation failures** become `Err(ApiError)`, which gives a JSON `error`, an exit code and one stderr
   line. **Per-item failures** inside a folder comparison become node states (`Error`, with `NodeError`s).
   They never fail the comparison (ARCH-101); they count in the statistics and make the verdict `Incomplete`
   unless important differences exist (D-02).
2. **Warnings** (`Vec<Warning>`) never change exit codes by themselves. Examples: budget exhaustion, lossy
   decoding, a missing capability for a criterion, a skipped user format, `closeness_ignored`,
   `rules_size_limit`, `switch_ignored`, `alignment_degraded`.
3. **Nothing is silent.** Every swallowed failure becomes a warning, a node error, or a counted
   `regex_errors`. A fault-injection test (`MemoryVfs::inject`, fake `ProcessRunner`) covers each injection
   point.
4. `Classify` mapping (selection): `NotFound` → `not_found`; `PermissionDenied` → `permission_denied`;
   `Unsupported` → `unsupported`; `TooLarge`/`TooManyLines` → `too_large`; `Cancelled` → `cancelled`;
   `Io{source}` → `io_error_kind(source)`; `FormatError::*` → `invalid_data`, or `invalid_input` for an unknown
   format or encoding, or `unsupported` for `FormatError::Schema` (a file from a newer version, as
   `SchemaError` itself, §1.1.12); `IpcError::Protocol`/`BadMessage`/`IncompatibleVersion` → `protocol`; `InvariantViolation`
   and panics → `internal`.

### 5.6 Panics

Engines are total (T9). As defence in depth, every `Service` method body runs inside
`run_guarded(|| …)` = `std::panic::catch_unwind(AssertUnwindSafe(…))`, and so does every `Service::spawn` job. A
panic becomes `ApiError::Internal("panic: <message>")` and exit code 100. A panic on a rayon worker propagates to
the `install`/`scope` caller and is caught there. A panic on the imara helper thread is reported through the
closed channel and becomes `ApiError::Internal`. *Amended in the wave-4 session review (§1.8.7 item 16):* the
service pool has a `panic_handler`, so a panic in a task given to `Service::pool().spawn` (which has no caller to
propagate to) is logged and ends that task only, instead of aborting the process; the IPC server answers a handler
panic with `Error{internal}`, and a waiter the panic dropped reports its view as `abandoned`. The CLI panic hook prints one line to stderr and the full
backtrace to the `--log` file. The release profile must keep `panic = "unwind"`; the architecture test asserts
it. Rust's default panic exit code 101 would collide with CLI-004 code 101 (D-35).

### 5.7 Memory bounds (accepted by the benchmarks in §6.4)

| Operation | Steady state | Peak (target) |
|---|---|---|
| Text compare (per side) | rope ≈ 1.05 × text bytes, plus 8 B/line line table, 4 B/line key ids, 1 B/line weights (Standard only), 8 B/line start states (multi-line grammars only) | raw bytes are dropped after `bytes_identical`; key arenas are transient (≤ text bytes + 8 B/line; *wave 3:* ≈ text bytes on both paths, worst case ≈ 2 × text + 2 B/line, §0.8). Total peak ≤ 3 × (L + R bytes) + 64 B × (L + R lines) + 32 MiB (gate G1; measured on the G1 input: 294 MiB of 427 MiB; inputs whose keys reach the worst case can exceed the formula). Unimportant-pattern matches are streamed (O(patterns) per line, never per match) |
| Rows | 12 B/row, plus sections | — |
| Closeness | — | ≤ `max_cells_per_gap` × 2 B (8 MB) per gap; one gap at a time; total cells ≤ `max_cells_total`; *wave 3:* the scorer's profile windows hold ≤ 2 × 2,048 samples of ≤ 256 chars (≤ 3 KiB each, ≈ 12 MiB worst case) |
| Folder compare | ≤ 256 B per node with both sides' metadata, excluding names (§1.6.6) → ≈ 26 MB + names for 100k pairs | plus in-flight directory batches |
| Content tests in folder compare | — | ≤ `content_memory_budget_mib` (default 1 GiB) of file content in flight across all tasks (D-25), plus the comparators' stream buffers |

---

## 6. Test plan, fixtures, benchmarks, and acceptance per wave

### 6.1 Test infrastructure (all crates)

- **Layout.** `core/<crate>/tests/<suite>.rs` (integration), `core/<crate>/tests/common/mod.rs` (golden helper
  and proptest strategies; there is no new shared crate, so the workspace layout does not change),
  `core/<crate>/tests/fixtures/<suite>/<case>/…` (golden inputs and outputs), `core/<crate>/benches/<name>.rs`
  (criterion; `[[bench]] harness = false`).
- **Golden helper.** `assert_golden(expected: &Path, actual: &[u8])` compares bytes and shows a
  `pretty_assertions` diff for text. With `HIKASA_BLESS=1` it rewrites the expected file instead (tests may read
  the environment; core `src/` may not). Engine outputs are deterministic by design, so engine goldens need
  **no normalisation**. CLI goldens replace the temporary directory with `<TMP>`, the version with `<VERSION>`,
  and drop the report `Generated:` line. *Amended in the wave-5 review (§1.9.3 item 4):* they also drop HTML's
  `Generated` header row, mask the machine's uid, gid and status-change times (`<UID>`, `<GID>`, `<CTIME>`),
  collapse the column padding after a replaced `<TMP>`, and drop the header line of a narrow side-by-side report
  that cuts the temporary folder at a length-dependent place; the completions golden replaces the script text
  with `<SCRIPT>` in the raw document (field order is checked).
- **Proptest.** 256 cases per property in normal CI (`PROPTEST_CASES` overrides) and 100,000 in a nightly job.
  Failing seeds are persisted under `tests/proptest-regressions/` and committed, so every regression becomes
  permanent (17-agent-workflow §3.2).
- **Deterministic generators.** Benchmark inputs are generated with a seeded xorshift64* (no `rand` crate). They
  are never committed.
- **Allocation counting.** Tests and benches that assert "no per-item allocation" install a counting
  `#[global_allocator]` in their own test binary (never in library code).
- **Folder fixtures.** Each `tree.toml` describes both sides: paths, kinds, contents (inline or generated),
  `mtime` (RFC 3339), attributes, symlinks and faults. The test materialises it into `MemoryVfs` (exact and
  portable) or into a tempdir with `filetime` (integration).
- **Architecture test** (`core/hikasa-session/tests/architecture.rs`; it reads the workspace sources via
  `CARGO_MANIFEST_DIR/../..`) checks:
  1. the `unsafe` keyword (regex of §1.2.6) appears only in `core/hikasa-vfs/src/mmap.rs`, and each occurrence
     has a preceding `// SAFETY:`. The one exemption is allocation counting (below): a file in a crate's own
     `tests/` or `benches/` directory (`<top>/<crate>/tests/…` or `<top>/<crate>/benches/…`, where `<top>` is
     `core`, `cli` or `gui`) whose code (not a comment or string) installs a `#[global_allocator]` may use
     `unsafe`, because `GlobalAlloc` cannot be implemented without it. Such a file must say
     `#![allow(unsafe_code)]`, and each `unsafe` in it still needs a preceding `// SAFETY:`. Library code never
     qualifies, not even a `src/tests/` module; library crates keep their crate-root lint (§0.4 rule 2:
     `forbid(unsafe_code)`, or in `hikasa-vfs` `deny(unsafe_code)` with the one `mmap` allow);
  2. no banned tokens in `core/*/src`: `std::env::`, `SystemTime::now`, `Instant::now`, `thread::sleep`,
     `thread_local!`, `static mut`, `println!`, `eprintln!`, `print!`, `dbg!` and `process::exit`. There are
     exactly three allow-points, each with a comment naming this rule:
     - `core/hikasa-core/src/progress.rs` may use `Instant::now` (`ThrottledSink`, which affects delivery only);
     - `core/hikasa-formats/src/convert.rs` may use `Instant::now` and `thread::sleep` (`StdProcessRunner`
       timeout polling);
     - `core/hikasa-session/src/host.rs` may use `std::env::` and the system clock (`capture_env`,
       `SystemClock`).

     Retry budgets are sums of injected sleeps, socket waits use `set_recv_timeout`, budget waits use
     `Condvar::wait_for`, and the imara helper uses `Receiver::recv_timeout`, so no further allow-point exists.
     `tracing::` may appear only in hikasa-session. `capture_env` and `SystemClock` are referenced only from
     `cli/` and `gui/`;
  3. every dependency in every member `Cargo.toml` is `workspace = true`, is listed in
     `[workspace.dependencies]`, and matches the §0.5 table. The dependency direction holds: no `core/*` crate
     depends on cli/gui, and the engine crates do not depend on session (ARCH-010/011/012);
  4. the stub crates export no `pub` items;
  5. `[profile.release]` does not set `panic = "abort"`;
  6. no `HashMap`/`HashSet` type appears in any DTO or persisted config struct (grep in `json/`, `dto.rs`,
     `protocol.rs`, `rules.rs` and `model.rs`);
  7. `cli/` and `gui/` contain no `impl FromStr` and no `match` over string literals of the core vocabularies
     (`--display`, `--criteria`, `--unimportant`, legacy switches): those parsers live in core (T11).

  *Amended in the wave-4 session review (decision 8).* The checks are a heuristic over the source text, hardened
  against the bypasses the reviewers demonstrated: `target/` and hidden folders are skipped only at a crate or
  workspace root (every folder below `src/` is scanned); `#[path = …]` and `include!` are not allowed in any
  member's `src/` (`include_str!`/`include_bytes!` stay allowed: data); check 1 counts every bare `unsafe` token
  (also `#[unsafe(no_mangle)]` and an `unsafe` handed to a macro); check 2 matches the banned tokens after removing
  whitespace and comments around `::`, unwrapping qualified self (`<Instant>::now`), resolving `use` aliases,
  crate aliases (`use std as s`, `extern crate std as s`), type aliases (`type I = Instant`) and the identifiers a
  same-file `macro_rules!` splices into a path; it adds `thread::park_timeout`, `io::stdout` and `io::stderr` to
  the banned list; a renaming re-export of `capture_env`/`SystemClock` is a violation; check 3 refuses member-level
  `features`/`default-features` (the feature sets of §0.5/§0.7 are fixed in `[workspace.dependencies]`); check 4
  also refuses `#[macro_export]`, `#[no_mangle]` and `#[export_name]` in stub crates; check 5 parses the workspace
  manifest and `.cargo/config.toml`/`.cargo/config` as TOML (every spelling, every profile); check 6 follows
  `HashMap`/`HashSet` type aliases across the core crates and looks into every `Serialize`/`Deserialize` item of
  every core source file, besides the DTO files; check 7 also flags vocabulary words in `matches!` patterns, as
  arguments of string comparisons (`eq`, `eq_ignore_ascii_case`, `starts_with`, `strip_prefix`, `contains`, …), in
  raw and escaped literals, in array tables and `const`/`static` items, in clap value declarations
  (`#[value(name = …)]`, `PossibleValue`, `value_parser = […]`), `FromStr` implemented under an imported alias, and
  clap `ValueEnum` enums whose variants spell vocabulary words (the CLI parses such words with the core parsers,
  §1.9.2). Each hardening has a self-test. Limits (not seen): code generated by procedural macros or build
  scripts, macros defined in another file, `CARGO_PROFILE_*` and other environment overrides, files outside the
  module tree reached at build time, and vocabulary assembled at run time (`format!`, concatenation).

### 6.2 Per-crate tests

#### 6.2.1 hikasa-core

- **Table-driven unit tests:** §2.1 (30 rows × relevant styles), §2.2.4 (37 rows), §2.3 (11 rows), the
  `ExitCode::for_content`/`for_kind`/`for_verdict` rows of §2.21, `FileTime` edge cases (pre-epoch, nanosecond
  rounding, the out-of-range `@secs.nanos` fallback, `TimeParseError`), `CancelToken` parent/child semantics,
  `Secret` never appearing in `Debug` or `canonical()`, `RelPath` validation, `Side` `Display`,
  `ContentKind`/`CompareVerdict` name round trips, and `Ctl::progress` (the closure is not called when the sink
  is not `due`).
- **Properties:**
  - P-SPEC-1: round trip `parse(canonical(s)) == s.without_secrets()` over generated components (Unicode,
    spaces, both styles).
  - P-SPEC-2: `normalize_local` is idempotent.
  - P-MASK-1, differential testing: Hikasa's matcher against an oracle that translates masks to `regex`
    (dev-dependency), on random masks over `{a, b, ., *, ?, [, ], !, -}` and random names, in both case modes;
    `matches(parent, name) == matches_path(parent.join(name))`.
  - P-MASK-2: `MaskList::parse ∘ to_list_string` is the identity.
  - P-MASK-3: case-insensitive matches are invariant under `fold(name)` for every *regular* name, i.e. one
    that meets both conditions of the test's `fold_is_regular`: (1) its case fold commutes with NFC
    (`nfc(fold(n)) == fold(nfc(n))` char by char), and (2) every char `c` of its NFC form is its own lower case
    or the upper case of its lower case (`c == simple_lower(c) || c == simple_upper(simple_lower(c))`). An
    irregular name matches exactly like its NFC form: `"I\u{307}"` (whose NFC form `İ` folds to itself) and
    `"W\u{30A}"` fail (1); `ǅ`, `ẞ` and `ϴ` commute with NFC but fail (2).
  - P-MASK-4: NFC invariance, `match(m, n) == match(m, nfd(n))`.
  - P-FILTER-1: adding an exclude mask never adds included items.
  - P-FILTER-2: `Excluded` propagates to every descendant.
  - P-TIME-1: `FileTime` serde round trip, and the text equals jiff's `Timestamp` Display.
  - P-CASE-1: `simple_lower` is idempotent and preserves char counts.
- **Allocation test:** `Mask::matches` and `NameFilter::file` allocate nothing for ASCII names (counting allocator).
- **Bench:** `mask_match` (gate in §6.4).

#### 6.2.2 hikasa-vfs

- **Unit tests:** `DosAttrs::letters`/`parse_letters` round trip and its serde form (`"HRS"`, D-30); the
  `permissions_string` table; `FileName::{extension, stem}` (`.bashrc`, `a.`, `a.b.c`, `x`); the router dispatch
  table (§1.2.4) including exact `Unsupported` messages and `name_hint`; `AtomicWrite` (commit is visible, drop
  discards, `overwrite = false` gives `AlreadyExists`); `ContentSource::read_all` (limit → `TooLarge`,
  cancellation → `ReadError::Cancelled`).
- **Prefilter:** `LocalVfs` and `MemoryVfs` report declined entries in `name_only` without stat (a counting
  `Vfs` wrapper over `MemoryVfs` proves no `stat` call); symlinks and unknown kinds are stat'ed (unknown kinds
  through `MemoryVfs::set_kind_unknown`; `LocalVfs`: §1.2.7 item 3).
- **mmap re-stat:** a test-only hook changes the file length between the mapped comparison and the re-stat; the
  result comes from streaming.
- **LocalVfs integration on Unix:** files, folders, symlinks, a broken symlink (`EntryError`), a non-UTF-8 name
  (raw round trip), and a permission-denied folder (skipped when running as root).
- **Properties:**
  - P-VFS-1: `compare_readers` equals naive whole-buffer comparison, including `first_difference`, for random
    contents and buffer sizes. A test-only path also exercises 1–64 byte buffers.
  - P-VFS-2: the mmap path equals the stream path (INV-VFS-2), for every thread count, also across many mapped
    chunks (a unit property with chunks of 1 byte to 64 KiB and 1–8 threads).
  - P-VFS-3: `crc32_reader` equals `crc32fast::hash`.
  - P-VFS-4: `MemoryVfs` listing orders all give the same entry sets.
- **Fault injection:** every `VfsOp` × `ErrorKind` combination surfaces the documented error.
- **Benches:** `binary_compare_2gb` (sparse tempfiles: identical, and differing in the last byte) and `crc32_1gb`.

#### 6.2.3 hikasa-diff

- **Golden fixtures:** `tests/fixtures/align/<case>/{left.txt, right.txt, case.toml, expected.<algorithm>.json}`.
  Each line is one token; `expected.*.json` holds the runs. At least 14 cases: empty/empty, empty/non-empty,
  identical, single change, moved block, repetitive `}`-heavy code, insertion at the start and at the end
  (tests skew windows), weighted headers, similar-line pairs (closeness for Standard, Histogram and Patience),
  Myers with closeness on (positional + `closeness_ignored`), never-align, patience fallback (no unique lines),
  and a budget fallback.
- **Properties** (alphabets of 1–8 symbols, lengths 0–200, random trivial sets and weights):
  - P-ALIGN-1: `check_invariants` holds for every algorithm and option combination.
  - P-ALIGN-2: `apply` reconstructs the right side.
  - P-ALIGN-3: identical inputs give one Equal run.
  - P-ALIGN-4: the number of Equal pairs is ≤ the LCS computed by an O(nm) DP oracle. A sanity check confirms
    the DP oracle agrees with `similar`'s LCS length.
  - P-ALIGN-5: never-align produces no Changed runs; Unaligned produces only (k, k) pairs.
  - P-ALIGN-6: Standard with `S = 0`, `n = m` and closeness off gives `i = j` for every Equal pair.
  - P-ALIGN-7: two runs give equal results, the result is the same with and without a `RunSink`, and the
    concatenated sink output equals `runs()`.
  - P-ALIGN-8: `work_budget = 0` gives the Histogram result, plus warning `work_budget_exhausted` whenever the
    stripped middle has both sides non-empty (property-sized inputs fit the fallback budget; §1.3.3 item 5).
  - P-ALIGN-9: `DenseRemap` preserves token equality and first-occurrence order, and every remapped id is
    `< k` (the debug assertion is active in tests).
  - P-ALIGN-10 (wave 2, §1.3.3 item 3): Standard with any work budget and any fallback budget (down to 0, through
    `testing::align_with_fallback_budget`, so small inputs reach the degraded split) keeps INV-ALIGN-1..7,
    reconstructs the right side, pairs at most the LCS and gives the same result with and without a `RunSink`;
    with both budgets 0, any work gives `alignment_degraded`.
  - P-CLOSE-1..3, including the brute-force optimality check for p, q ≤ 6, for Standard, Histogram and Patience.
  - P-SIM-1: bigram similarity is symmetric, lies in [0, 1000], and is 1000 iff the multisets are equal.
  - P-INLINE-1..4.
  - P-INTERN-1: interned ids are dense and in first-seen order.
- **Unit tests:** `MAX_SEQUENCE_LEN` boundary (`len = MAX_SEQUENCE_LEN + 1` → `InputTooLarge`, checked by the
  length guard without allocating such a sequence); the helper-thread path (a test lowers the threshold through a
  test-only option and cancels mid-call: `Err(Cancelled)` within 100 ms). Added in wave 2 (§1.3.3): bounded
  Standard (`bounded_standard`: 1,000,000 random lines over 50 symbols < 2 s in release, ×8 the input < ×32 the
  time in debug; the degraded split and the pairing of a residual gap beyond the fallback budget); the cap on
  abandoned helpers (`abandoned_helpers`, its own test binary); single-shot cancellation inside a 1 × 4,000,000
  closeness gap; cancellable linear scans; Myers and Histogram send the common prefix first.
- **Benches:** `align_1m_histogram`, `align_1m_standard` (1M lines, 1 % changes), `align_pathological`
  (≥ 10,000 anchor-less gaps on highly repetitive input; checks DenseRemap and the budget fallback time),
  `align_random_standard` (wave 2: 1M random lines over 50 symbols; checks the bounded fallback).

#### 6.2.4 hikasa-formats

- **Unit tests:**
  - every preset parses, compiles and round-trips through `to_toml` (INV-FMT-1), starts with a `# Sources:`
    comment, and has a non-empty keyword list where Appendix A gives one;
  - preset order and enabled flags (D-26, D-27);
  - schema checks: a newer schema is an error, an unknown key is a warning;
  - precise `RawGrammarItem` errors;
  - element ids (D-41): a format whose `[elements]` lexicographic order differs from first use in `grammar`
    gets ids by first use; unused declared elements come last by name; `important = false` reaches
    `ElementTable::default_important`;
  - regex engine selection (`a+` → Standard, `(a)\1` → Fancy, `(?<=x)y` → Fancy, `(` → error carrying both
    messages);
  - the escape table (§2.10.3);
  - tokenizer edge cases: empty line, zero-length regex, columns past EOL, Lines with `or_line_one`, longest
    List match, word boundaries, `word_chars` (COBOL `END-IF` keyword, `END-OF-FILE` identifier), Java
    `non-sealed`, Unicode identifiers; field boundaries of Columns items (COBOL columns 73–80 behind a literal
    or an identifier that reaches column 72); runtime-limit hits that must not hide a later match (fast paths
    and reference agree on tokens and error counts);
  - long single lines (wave-2 review, `tests/long_lines.rs`): the adversarial lines of Python, Bash, Rust and
    Visual Basic tokenise in linear time (a scaling ratio in debug builds, 1 MiB in < 500 ms in release), and a
    cancel during one 1 MiB line returns within 100 ms (G4), sequentially and on a pool;
  - `build_argv` table;
  - `run_external` with a fake runner (exit codes, empty output, timeout, cancellation), and on Unix with `cat`
    and `sort`.
- **Golden fixtures:**
  - `tests/fixtures/tokens/<preset-id>/<case>.<ext>` + `<case>.tokens.txt` (the same text format as
    `hikasa format tokens`), with at least two cases per text preset: a typical file and a tricky one;
  - `tests/fixtures/encoding/<case>.bin` + `<case>.expected.toml`: every row of the §2.8 table (each BOM,
    BOM-less ASCII UTF-16LE and UTF-16BE, BOM-less UTF-32LE, windows-1252, Shift_JIS, ASCII with and without a
    fallback, invalid UTF-8, an override);
  - `tests/fixtures/binary/<case>`: each magic kind, UTF-16 text, text with ESC sequences, an 11 % control-byte
    buffer.
- **Properties:**
  - P-TOK-1..3: token invariants on random grammars from a small item generator, over random lines.
  - P-TOK-4: re-tokenising any line from its stored start state reproduces the tokens of the full pass
    (INV-TOK-4; this is what `TextComparison::spans` relies on).
  - P-TOK-5: parallel equals sequential for pool sizes 1, 3 and 8, including patched chunk prefixes. A test-only
    `tokenize_all_with_chunk` forces chunks of 4 lines.
  - P-ENC-1: decoding round-trips. P-ENC-2: `detect` and `decode` are total over arbitrary bytes.
  - P-ENC-3: `detect` and `sniff` agree on every input that `utf_pattern` accepts.
  - P-SHADOW-1: shadowing soundness.
  - P-ARGV-1: substituted values are never split.
- **Bench:** `tokenize_c_1m` (throughput and allocation count, §6.4).

#### 6.2.5 hikasa-text

- **Unit tests:**
  - line splitting (`""`, `"\n"`, `"a"`, `"a\r\nb\rc\n"`, `"\r\r\n"`), `EolSummary::style`, and CPL wrapping;
  - the line-count guard (`MAX_LINES + 1` → `TooManyLines`, tested on the guard, not on a 2 GiB buffer);
  - the Sorted conversion (D-38);
  - every row of the key table §2.13.3;
  - `UnimportantItem::parse_list` (each word, `element:<Name>`, empty items, invalid names) and
    `ImportanceRules::apply_unimportant`;
  - `TextRules::is_identity` for the defaults with every preset (identity for formats without unimportant or
    case-insensitive elements);
  - row, section and statistics computation on hand-built alignments;
  - the verdict rules of §2.17.5 (incl. D-12 binary content);
  - the view filter table (each filter, with iu on and off, with `expanded` ranges);
  - `thumbnail` (buckets fewer and more than rows; iu on and off);
  - inline classification: a case-only SQL keyword change is unimportant, and a comment change inside an
    important line is blue when comments are unimportant.
  - *Wave 3 (review regressions, `tests/regressions.rs` and unit tests):* the key encoding (identity key = general
    key in every tokenizer state; `int x` ≠ `intx`, `a + b` = `a+b` under ws-off; end markers); the streamed pattern
    union equals the sorted union and stops on cancellation; the scorer values (`x  `/`x` 999, `}`/`};` 666,
    `x`/`y` 0) and its branch-free Dice equals `bigram_similarity`; short lines and braces pair as Changed rows
    with every closeness engine; byte-identical lines are never Important; the quick step-5 guard; an inserted line
    break never equals an artificial wrap; Changed/Unimportant rows are all blue; `Auto` resolution (D-36); the
    scorer budget (`closeness_budget_exhausted`, pool-independent); External conversions and regex limits agree on
    every path (fake runner); `lossy_decode` once per side; a right-side format compile error; stream options;
    `inline()`/`spans()` bounds on long lines and equality with `inline_diff`; chunked decoding equals the one-shot
    decode for nine encodings and piece sizes 1–64; the cancellable Sorted conversion equals `sort_lines`; the
    D-43 guard counts wrapped pieces; a cancel during decoding is `Err(Cancelled)`; `Everything Else` handling; a
    truncated document warns.
- **Golden fixtures:** `tests/fixtures/compare/<case>/{left.<ext>, right.<ext>, rules.toml?, expected.json}`.
  `expected.json` is the canonical serialisation of runs, rows (run-length compressed), sections, statistics,
  verdict and warnings. There are at least 25 cases, covering every row of §2.13.3 and every algorithm.
- **Properties:**
  - P-TC-1: `check_invariants` on random documents with whitespace and comment variants, under random rules.
  - P-TC-2: the consistency theorem, `quick_compare_bytes == compare().verdict(false)`, for every algorithm and
    both values of `orphans_always_important` (covers the step-5 shortcut).
  - P-TC-3: pool `None` gives the same result as pool 8.
  - P-TC-4: view invariants, and Context visibility equals the brute-force definition. *Wave 3:* the thumbnail
    cells equal a per-row recount (contents, not only the length).
  - P-TC-5: statistics identities (`rows = same + changed + left_only + right_only`, and so on).
  - P-TC-6: ignore-unimportant only ever makes the verdict "more same".
  - P-TC-7: the identity fast path equals the general path (`force_tokenize`) for random texts under rules for
    which `is_identity` holds (D-53). *Wave 3:* also for grammars with multi-line items (Rust, XML block comments)
    and for the general path forced by a never-matching unimportant pattern.
  - P-TC-8: the runs passed to `CompareObserver::partial`, concatenated, form a prefix of the final runs, and the
    observer never changes the result.
- **Benches:** `text_compare_1m_auto`, `text_compare_1m_standard`, `text_compare_1m_identity` (1M lines, about
  1 % changes, C-like); `gui_enablers` (G5). `perf_rss` is an `#[ignore]` test that reads `VmHWM` from
  `/proc/self/status` (Linux) and asserts G1's memory bound. *Wave 3:* the `#[ignore]` gate
  `g4_cancel_latency_adversarial` (tests/gates.rs) measures G4 through `load_document` and `compare` on the review's
  adversarial inputs (64 × 200,000-char and 2,000 × 2,000-char unrelated lines, one 100 MiB line per side on the
  identity and C paths with `inline`/`spans` timings, a dense unimportant pattern on 50 MiB lines in compare and
  quick, 256 MiB documents in UTF-8, UTF-16LE and windows-1252 being decoded, 2 × 7 MiB repetitive C with `Auto`
  under 3 s), with total times and peak memory. The global-pool trap keeps every global-pool thread busy, so an
  implicit global-pool use in any pass (line weights and the pattern runtime-limit pass included) blocks and fails.

#### 6.2.6 hikasa-folder

- **Unit tests:** the tables of §2.4 (13 rows), §2.5.2 (9), §2.6.2 (15), §2.6.6 (7), the category and colour
  tables (§2.6.3/§2.6.4) and §2.7 (13 presets); `SizeFilter`/`DateFilter` parsing, including errors and zones;
  `ByteBudget` (never more than the capacity in flight; a request larger than the capacity takes the whole
  capacity; cancellation while waiting returns within 20 ms); `Change<T>` JSON round trip (`"clear"` survives);
  verdict cases.
- **Golden fixtures:** `tests/fixtures/trees/<case>/{tree.toml, rules.toml, expected.json}`. `expected.json`
  holds `CanonicalTree`, statistics and verdict. There are at least 22 cases: identical trees, newer files,
  orphans, nested orphans, case-only names in both modes, NFC/NFD, extension alignment, kind mismatch,
  symlinks (follow on/off, and a loop), every row of §2.3, other filters, flatten, CRC/binary content, rules
  content with a scripted fake comparator (incl. `rules_size_limit`), the D-48 rows, a one-sided listing
  failure, fault injection (an unreadable subfolder, a stat error), and cancellation part-way through. Wave 3 adds
  (§1.6.8): a folder protected on one side only, on every side and as an orphan, in both scan modes; a listing
  failure in flatten mode; an unlistable orphan folder (Error, not an orphan); excluded broken entries (Filtered
  with their errors); a link cycle reported as `ELOOP`, in both scan modes (45 cases in all).
- **Properties:**
  - P-FOLDER-1: `check_invariants` on random tree pairs.
  - P-FOLDER-2: the canonical tree is identical across pools (None/1/8) × listing orders
    (Sorted/Reversed/Shuffled).
  - P-FOLDER-3: swapping the sides mirrors the result (orphans and newer swap) for content methods
    None/CRC/Binary.
  - P-FOLDER-4: replaying the events against the live model (re-reading the named nodes, the children of
    listed folders and the ancestors of both at each event) ends with exactly `to_canonical()` (INV-EVT-3, as
    amended in §1.6.8 item 21).
  - P-FOLDER-5: more exclusions never add visible nodes (display All; Differences only when no FC-054 pair can
    arise, §1.6.8 item 13).
  - P-FOLDER-6: the counters and totals equal a recount after every applied batch (a test observer takes a
    `TreeRead` at each event and checks INV-FOLDER-5 on it), and the statistics equal a recount at the end.
  - P-FOLDER-7: `visible_rows` equals a recursive reference implementation of §2.7 for every filter, mode,
    iu and suppress value, on complete and partial trees.
  - P-FOLDER-8: ignore-unimportant monotonicity (INV-FOLDER-10).
  - P-RULES-2: `FolderRules` TOML round trip over generated values of **every** field (incl.
    `timestamps.enabled = false`, `offset`, size and date filters, DOS letters) (INV-RULES-2).
- **Prefilter:** a counting `Vfs` wrapper proves that excluded names are not stat'ed, and that the filtered side
  of an FC-054 pair is stat'ed exactly once.
- **Integration (LocalVfs + tempdir):** real mtimes through `filetime`, Unix permissions, a symlink loop (a link
  back to an ancestor and a self link reported as `ELOOP`, both with `SYMLINK_LOOP`), and non-UTF-8 names
  (including two different ones that must not pair under case-insensitive alignment).
- **Regressions (wave 3, `tests/regressions.rs`):** a panic in the scan leaves the model finished and incomplete;
  a cancellation right after a 200,000-entry folder was listed, or while its batch is planned, drops the batch
  (no partial insertion, `complete == false`); a cancellation at any moment keeps batches whole; an FC-054
  re-stat storm stops at the cancellation (no stat after it); 20,000 entries whose stat failed pair in linear
  time; quick tests on filtered stat'ed pairs; a one-sided protected folder gives NoDifferences in both modes.
- **Benches:** `folder_scan_100k` (generated in a tempdir; identical trees, and trees with 1 % differences);
  `folder_model_1m` (a 1M-entry pair generated in `MemoryVfs`; memory per node); `cancel_latency` (G4: the 100k
  scan cancelled half-way; one folder of 1,000,000 entries per side cancelled right after its listing and at
  delays that land in every step of its batch; 200,000 FC-054 re-stats cancelled half-way). The benches assert
  their gates (G2 warm on the fastest of five runs, G4 on the worst latency, `folder_model_1m`);
  `HIKASA_GATES=report` only prints them (noisy developer machines).

#### 6.2.7 hikasa-report

- **Golden fixtures:** these reuse the text and folder cases. Text cases produce `expected.annotated.diff`,
  `expected.unified.patch`, `expected.context.patch`, `expected.normal.patch`, `expected.sbs.txt`,
  `expected.sbs.html`, `expected.summary.txt` and `expected.stats.csv`. Folder cases produce `expected.list.txt`,
  `expected.sbs.txt`, `expected.sbs.html`, `expected.summary.txt` and `expected.json`. *Wave 4:* also
  `expected.summary.html` and the text `expected.json`; ANSI goldens for some cases (§1.7.3); the goldens are
  ignored, visibly, off Unix (they hold Unix host paths).
- **Properties:**
  - P-RPT-1: every patch reproduces the right side from the left one. (a) Generated LF documents with
    `compare_line_endings = true` and `ignore_unimportant = false`: `diffy::apply(left, parse(unified(cmp))) ==
    right` for context ≥ 1 (wave 4: `diffy` mis-places zero-context deletions of duplicate lines), and every style
    at every context with position-exact, byte-exact appliers written in the tests. (b) *Wave 4:* the default
    rules (line endings not compared, or compared) with random LF, CRLF and lone-CR endings, missing final
    newlines and BOMs; byte-exact for every style (lone CR as LF, warned), and `\ No newline at end of file` only
    after the last line of a side. A fixed corpus is also applied with GNU patch (no fuzz, every style) and `git
    apply` (unified) when they are on PATH.
  - P-RPT-2: rendering twice gives identical bytes.
  - P-RPT-3: the HTML is well-formed according to a small tag-balance and escaping checker.
- **Unit tests:** HTML, CSV and control-character escaping; hunk header formatting (`,1` omitted; `s = 0`);
  `ReportMeta::for_text`/`for_folder` wording; the RPT-004 header lines (mode, rules, per-side errors).

#### 6.2.8 hikasa-session

- **Unit tests:** `resolve_config_dir` and the cache directory (explicit, portable, each platform, missing
  pieces, `is_default`); `Endpoint::for_config` (every row of §2.23, the long-path fallback, the override);
  `fnv1a64` known vectors; `load_options` (missing, partial, an unknown key inside a known section → silent and
  listed in `ignored_keys`, an unknown top-level section → `unknown_config_key`, newer schema);
  `FormatStore::load` (overriding a built-in, a new user format, `order.toml` with unknown ids, a broken file);
  `write_atomic`; `ApiError::exit_code` (every row of §2.21, both `wait` values); `infer_view_kind`;
  `gui_exe_candidates`; `normalize_legacy_args` and `parse_open_args` tables (`-ro`, `/ro` on Windows vs a path
  on Unix, aliases, `--name value` and `--name=value`, forwarded switches with `switch_ignored`, unknown switches
  → `InvalidArgument`, `-` and `clipboard://` handling).
- **IPC:**
  - framing properties: arbitrary payloads up to 64 KiB round-trip; a 1-byte-at-a-time reader; oversized frames
    are rejected; a truncated frame gives `ConnectionLost`;
  - serde round trips for every message, plus the wire examples of §4.4.3 as goldens; unknown fields are
    ignored; an unknown `type` gives `unsupported_request` with the same id; a known type with bad fields gives
    `invalid_request` with the same id; a frame without `id` gives `protocol` with id 0; `ViewKind::Unknown` gives
    `unsupported_view`; a client receiving `Response::Unknown` reports a protocol error;
  - a Unix-socket integration test (in a tempdir, via `HIKASA_IPC_ENDPOINT`-style overrides in `EnvFacts`)
    covers: the handshake, version negotiation (client 2/min 1 vs server 1 → 1; client min 2 → incompatible),
    `OpenView{wait}` → `ViewClosed`, a dropped connection → `ConnectionLost`, `InstanceLock` exclusivity (and two
    configurations holding two locks), the 0700 folder check, the peer-uid check, and `ServerHandle::shutdown`
    returning within 1 s through the self-connect;
  - `connect_or_launch` with a fake launcher: the sum of recorded sleeps never exceeds `RetryPolicy::total`;
    `NotFound` → `GuiNotFound`; exhaustion → `LaunchTimeout`.
- **Service integration** (tempdir + LocalVfs + `FixedClock` + fake runner): quick, text and folder compare;
  `folder_prepare` + `folder_run` with a `ChannelObserver`; `gui_open` spooling `-` and deleting the spool after
  `--wait`. **End-to-end injection:** `folder compare --content rules` gives, for every pair, the same result as
  `quick` on that pair.
- **Jobs:** `Service::spawn` returns results, drops progress when the receiver is slow, turns a panic into
  `ApiError::Internal`, and cancels through `JobHandle::cancel`.
- **Panic boundary:** a comparator that panics yields `ApiError::Internal` and exit code 100.
- **The architecture test** (§6.1).
- *Wave-4 review regressions* (`tests/ipc_review.rs`, `tests/ipc_memory.rs`, `tests/review.rs`): the client
  refuses foreign-looking socket folders (wrong mode, another owner, a symbolic link, an unknown uid) and never
  launches into them; the server creates its folder 0700, refuses foreign ones and never deletes a regular file at
  the socket path; a dropped waiter → `abandoned`/100; shutdown notifies waiters and closes connections;
  `view_closed` does not block on a stalled client; a protocol error closes a connection with a live waiter; a
  chatty server cannot defeat the call deadline; the parked queue is bounded; an id-0 error fails the pending call;
  a 16 MiB frame of tiny values is refused without memory amplification; `pool().spawn` panics are not fatal; the
  effective folder ignore-unimportant in JSON and reports; the private stdin spool and its cleanup; the launch
  arguments and a launched GUI serving an explicit configuration; `HIKASA_GUI_EXE`; per-field unix criteria; the
  `--gui` ↔ `OpenView` mappings; local validation of `--gui`; lazy standard input; the D-23 close rules; FC-062
  with non-UTF-8 names; second-instance forwarding; the usage text; rules-file errors with the file's spec;
  `options.toml` edge cases; non-UTF-8 inline switch values and unknown Windows `/switch`es; empty line ranges;
  the envelope helpers; the grammar-dependent report paths; a newer-schema user format skipped.

### 6.3 CLI integration tests (`cli/hikasa-cli/tests/`, assert_cmd + predicates)

| File | Cases |
|---|---|
| `cli_quick.rs` | each exit code 1/2/11/12/13/100 on fixture pairs; `--json` golden; `--ignore-unimportant` (12 → 2); stdin `-` with `--stdin-name x.c` (C grammar used); a missing file gives 100; an unsupported scheme gives 100 with `error.kind = "unsupported"`; `--unimportant whitespace` turns 13 into 12 for a whitespace-only change |
| `cli_text.rs` | annotated unified golden (default); side-by-side text and HTML (`--width`, `--wrap`); each patch style; statistics CSV; `--output` is atomic (target untouched on error); 100 for a missing file; `--json` golden (default display = mismatches; `--context 2`); each `--algorithm`; `--rules-file`; `--left-encoding`; `text compare - x.c` detects C (D-15); `--gui` against the fake server |
| `cli_folder.rs` | list golden; `--json` golden; exit codes 2/12/13/107/100; `--filters`; `--criteria` (§2.6.6 rows); each `--display` preset; `--suppress-filters`; `--flatten`; `--content crc\|binary\|rules`; `--report side-by-side --html --output`; `--gui` |
| `cli_format.rs` | list/show/detect/tokens goldens, with `--config-dir` holding a user format that overrides a built-in; `format detect - --stdin-name a.py` |
| `cli_vfs.rs` | ls/stat/cat goldens; `cat --json` base64 |
| `cli_gui.rs` | `gui ping` with no GUI gives 100 (endpoint override → a tempdir socket); a scripted fake GUI server (test support binary built on `hikasa_session::ipc::server`) makes `gui open a b --wait` return the scripted code; `--no-launch` gives 100, `--no-launch --wait` gives 102; a server that drops the connection while waiting gives 102 (a fake-GUI process that exits: an in-process server answers a dropped waiter with `abandoned`/100); launch with a fake GUI through `HIKASA_GUI_EXE` (Unix; the override is checked before the sibling of the CLI, §1.8.7 item 18); 103 when `HIKASA_GUI_EXE` names a missing file; one launch without `HIKASA_IPC_ENDPOINT` (`XDG_RUNTIME_DIR` in a private temporary folder) with `--config-dir`, which must reach the launched GUI; legacy spellings (`-ro`, `--lefttitle=x`); `gui open clipboard://` gives 100; `--gui` with a bad `--criteria` gives 100 without contacting the GUI |
| `cli_global.rs` | a usage error gives 100 (not 2); `--help` and `--version` give 0; `--quiet` leaves stdout unchanged; a broken pipe keeps the exit code; `--no-color` gives no ANSI; `--log` creates the file; `--verbose` logs ignored option keys; JSON on error is still one document |

### 6.4 Benchmarks and performance gates

The benches use criterion. CI runs them on a fixed runner, and a regression of more than 10 % needs a
justification (17-agent-workflow §3.3). Gates assume a release build on a 2020-class 8-thread laptop, warm cache
unless stated, headless (§0.8).

| Gate | Bench | Crate | Threshold |
|---|---|---|---|
| G1 | `text_compare_1m_auto`, `text_compare_1m_standard`, `text_compare_1m_identity` | text | < 3 s end-to-end from bytes; peak RSS ≤ 3 × (L + R bytes) + 64 B × (L + R lines) + 32 MiB (`perf_rss`) |
| G2 | `folder_scan_100k` | folder | < 5 s scan + quick tests cold on SSD (perf runner with dropped caches); < 1.5 s warm (CI) |
| G3 | `binary_compare_2gb`, `crc32_1gb` | vfs | ≥ 90 % of the sequential-read throughput of the same files (warm page cache of a disk file system, not tmpfs; the mapped path with `StreamOptions::threads = 4`, the session's configuration; the baseline is one thread of 1 MiB `read` calls; the 1-thread figures are reported alongside) |
| G4 | `cancel_latency` (cancel at 50 % of G1 and of G2; also a large imara call) | text, folder, diff | ≤ 100 ms from `cancel()` to return |
| G5 | `gui_enablers` on the G1 result | text | `TextComparison::row` < 200 ns, `FilteredView::get` < 200 ns, `TextComparison::inline` p99 < 50 µs |
| — | `align_1m_histogram` / `align_1m_standard` | diff | < 0.3 s / < 1.0 s |
| — | `align_pathological` (≥ 10,000 anchor-less gaps) | diff | < 1.0 s; allocated bytes O(n + m) (DenseRemap) |
| — | `align_random_standard` (1,000,000 random lines per side over 50 symbols; wave 2) | diff | < 2.0 s (bounded Standard, §2.14.5) |
| — | `tokenize_c_1m` | formats | ≥ 50 MB/s per core; parallel scaling ≥ 0.6 × cores (up to 8); allocations ≤ 4 per chunk |
| — | `mask_match` | core | ≥ 20 M matches/s on a 20-item list (one match = one name checked against the whole list); zero allocations for ASCII names |
| — | `folder_model_1m` | folder | ≤ 256 B per node with both sides' metadata, excluding name bytes |

### 6.5 Implementation waves and acceptance criteria

Every wave ends green on: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo doc --workspace --no-deps` (with `deny(missing_docs)`), `cargo deny check`, and
the architecture test as far as it applies. Every PR names its requirement IDs (17-agent-workflow §1.1) and
carries the doc updates of §0.9 for its wave.

| Wave | Scope (parallel agents) | Acceptance criteria |
|---|---|---|
| **1 — core** | hikasa-core (1 agent) | A1.1 every §1.1 item implemented and documented; the §0.5 Cargo changes of hikasa-core. A1.2 table tests for §2.1, §2.2.4, §2.3 and the content/verdict rows of §2.21 pass. A1.3 P-SPEC-1/2, P-MASK-1..4, P-FILTER-1/2, P-TIME-1, P-CASE-1 pass with 4096 cases. A1.4 no `unsafe` in library code (the counting `#[global_allocator]` of a test or bench binary is the §6.1 exemption); no dependencies beyond §0.5; the `mask_match` gate and the ASCII zero-allocation test. A1.5 the architecture test (`core/hikasa-session/tests/architecture.rs`; the stub crate can hold tests) lands with checks 1, 2, 4 and 5; checks 3, 6 and 7 are added in wave 4, when every manifest has its final form. A1.6 wave-1 doc updates of §0.9. |
| **2 — vfs, diff, formats** | three agents; each depends on core only | **vfs**: §1.2 complete; P-VFS-1..4; router table; prefilter and re-stat tests; LocalVfs integration; `mmap.rs` is the only `unsafe` (check 1); G3. **diff**: §1.3 complete; P-ALIGN-1..9, P-CLOSE-1..3, P-SIM-1, P-INLINE-1..4, P-INTERN-1; ≥ 14 golden cases × 5 algorithms; `align_1m_*` and `align_pathological` gates; helper-thread cancellation test. **formats**: §1.4 complete; all 21 presets parse, compile and round-trip, each with a `# Sources:` comment and the Appendix A keyword lists, written from language references; a reviewer confirms independence from other products (17-agent-workflow §5.1); ≥ 2 token goldens per text preset; encoding and binary fixtures (§2.8 table, magic kinds); P-TOK-1..5, P-ENC-1..3, P-SHADOW-1, P-ARGV-1; `tokenize_c_1m` gate. End of wave: `cargo test --workspace` green; wave-2 doc updates. |
| **3 — text, folder** | two agents | **text**: §1.5 complete; the §2.13.3 table; ≥ 25 compare goldens; P-TC-1..8 including the consistency theorem and the identity equivalence; G1, G5 and the text part of G4. **folder**: §1.6 complete; the tables of §2.4–§2.7; ≥ 22 tree goldens; P-FOLDER-1..8, P-RULES-2; prefilter counting test; LocalVfs integration; G2, `folder_model_1m` and the folder part of G4. Rules content uses a fake comparator here; the real one is checked in wave 4. Wave-3 doc updates. |
| **4 — report, session** | two agents; the report API skeleton is merged first | **report**: §1.7 complete; goldens for every layout and format; P-RPT-1..3; RPT-004 header tests. **session**: §1.8 complete; config, cache, endpoint, options and format-store tables; legacy argument tables; IPC framing, protocol (incl. forward-compatibility goldens) and socket integration; jobs; Service integration including the end-to-end `RulesComparator` injection and `folder_prepare`/`folder_run`; the panic boundary; the full architecture test (checks 1–7). Wave-4 doc updates. |
| **5 — cli** | one agent | §1.9 complete; every file in §6.3; the exit-code table end to end; a JSON golden for every schema in §4.3; `completions` non-empty for all 5 shells; G1 and G2 through the binary in the perf job; `docs/12` updated with the M1 flag set and the D-entries of §0.9; CHANGELOG entry. M1 roadmap check: golden diffs ✓, 100k scan < 5 s ✓. |
| **6 — GUI prototype (outline)** | one or two agents, after the maintainer approves the GUI toolkit dependency (D-56) and `docs/02` records it | Scope (16-roadmap M1): one window; Folder Compare and Text Compare views; virtualised text panes drawn on a canvas (ARCH-GUI-002); the IPC server; `hikasa gui open … --wait`. The GUI consumes only §1.10 contracts and contains no comparison logic (architecture check 7; ARCH-L3). Acceptance: (a) ARCH-GUI-003 measured — scrolling a 1M-line side-by-side comparison with syntax colouring at ≥ 60 fps, < 400 MB RSS, first screen < 1 s — and **reported to the maintainer before any toolkit switch** (17-agent-workflow §7.1); (b) a 100k-file folder comparison renders incrementally while scanning with UI frames < 16 ms; (c) `hikasa gui open a b --wait` returns the view's exit code, `hikasa gui ping/list/close/quit` work, and a second `hikasa-gui a b` forwards to the running instance; (d) view-model logic is tested through the command catalogue (17-agent-workflow §3.4), not screenshots; (e) wave-6 doc updates. |

---

## 7. Requirement traceability

Status column: **M1** = implemented and tested in M1; **data** = core provides the data/API, the GUI renders it
in wave 6 or M2; **reserved** = types reserved, behaviour later; **gap** = blocked by an approved-dependency
decision (§0.7, Appendix C).

| Requirement | Crate::module | Verified by | Status |
|---|---|---|---|
| ARCH-L1, ARCH-012 | all core crates; thin `cli`/`gui` | architecture test (§6.1, checks 2 and 7) | M1 |
| ARCH-L2, CLI-000 | `hikasa-cli` (`args`, `output`) | §6.3 `cli_global.rs` | M1 |
| ARCH-L3 | `hikasa-gui` contract (§1.10); shared `parse_open_args` (D-51) | wave 6; check 7 | data |
| ARCH-L4, ARCH-091, ARCH-092, CLI-G01, CLI-G02, CLI-010 | `session::ipc`, `session::api::gui`, `cli::cmd::gui` | §6.2.8 IPC, §6.3 `cli_gui.rs` | M1 (client + server helpers) |
| CLI-G03 | `cli::cmd::{text, folder}` `--gui` → `Service::gui_open` (D-50) | `cli_text.rs`, `cli_folder.rs` | M1 |
| ARCH-001, ARCH-010, ARCH-011 | workspace `Cargo.toml` files | architecture test check 3 | M1 |
| ARCH-002 | `diff`, `text::compare`, `folder` (I/O only through `Vfs`/`ContentSource`) | purity bans (check 2) | M1 |
| ARCH-003, ARCH-060 | `core::{cancel, progress, job}`, `session::{JobControl, JobHandle, Service::spawn}` | G4, unit tests | M1 |
| ARCH-004, ARCH-082 | `diff::RunSink`, `text::CompareObserver`, `folder::{FolderModel, ScanEvent, ChannelObserver}` | P-ALIGN-7, P-TC-8, P-FOLDER-4/6 | M1 |
| ARCH-005 | `vfs::mmap` | architecture test check 1 | M1 |
| ARCH-006 (P1) | `core::{ErrorKind, WarningCode}` (stable codes for later localisation) | — | reserved |
| ARCH-007, SES-052 | `formats::toml_io`, `text::rules`, `folder::rules`, `session::options`, `core::schema` | schema unit tests, P-RULES-2, INV-FMT-1 | M1 |
| ARCH-013 | `session::api::Service` | §6.2.8 | M1 |
| ARCH-020, VFS-010 | `core::spec` | §2.1 table, P-SPEC-1/2 | M1 |
| ARCH-021 | `core::spec::Spec::ArchiveMember` (reserved) | M2 | reserved |
| ARCH-030, VFS-001 | `vfs::{caps, vfs}` | router table | M1 |
| ARCH-050 | run index (`diff`), `text::SectionId`, `folder::NodeId` | P-ALIGN-7 | M1 |
| ARCH-061 | service pool builder (the single place a priority would be applied) | — | gap (§0.7) |
| ARCH-080, ARCH-081 | `session` pool; `vfs::mmap`; `text` rope | §5.1 global-pool trap, P-VFS-2 | M1 |
| ARCH-090 | `session::ipc::{Endpoint, InstanceLock}` (per configuration, D-47) | IPC integration, §2.23 table | M1 |
| ARCH-100, ARCH-101 | all error enums, `core::ErrorReport`, folder `NodeError`s | fault injection | M1 |
| FC-001 | `folder::compare_into` | tree goldens | M1 |
| FC-010 | `folder::align` | §2.5.2 table | M1 |
| FC-012 (headless subset) | `folder::Handling::scan_orphan_folders`, `ScanStatus` | tree goldens | M1 |
| FC-013 | `folder` + `vfs::FollowLinks` | symlink goldens | M1 |
| FC-014 | `folder` flatten, `EntryView::dir` | flatten golden | M1 |
| FC-016 | `folder::scan` (parallel, cancellable, live model) | P-FOLDER-2, cancellation golden, G4 | M1 |
| FC-020 … FC-024 | `folder::{quick, time_cmp}` | §2.4 and §2.6.1 tables | M1 |
| FC-025 | `vfs::compare`, `text::quick::RulesComparator`, `core::content`, `folder::ByteBudget` | P-VFS-1..3, P-TC-2, end-to-end injection | M1 |
| FC-026, FC-027 | `folder::state` (D-48) | §2.6.2 table | M1 |
| FC-029 (a) | `folder::scan` | goldens ((b), (c) are GUI, M2) | M1 |
| FC-030, FC-031, FC-032 | `folder::{state, counts, model}`, `TreeRead::colour`, `core::ContentKind` | §2.6.3/§2.6.4 tables | M1 |
| FC-040 | `folder::EntryView`, `TreeRead::totals`, `report::folder` columns | report goldens | M1 |
| FC-050, FC-051 | `core::{mask, filter}` | §2.2.4 and §2.3 tables, P-MASK-*, P-FILTER-* | M1 |
| FC-053 (size, date, protected) | `folder::rules::OtherFilters` | filter goldens | M1 |
| FC-054 | `folder` per-side `filtered` (FC-054 pairs stat'ed after alignment) | §2.3 row 10 | M1 |
| FC-055, FC-056, FC-057 | `folder::display`, `TreeRead::visible_rows` (D-44) | §2.7 table, P-FOLDER-7 | M1 |
| FC-064 | `folder::stats` (root counters) | P-FOLDER-6 | M1 |
| TC-001, FF-032 | `text::document`, `formats::encoding` (per-side override) | encoding fixtures, P-ENC-* | M1 |
| TC-002 (local, stdin) | `text::load_document`, `vfs::VfsRouter` | `cli_quick.rs` stdin | M1 |
| TC-003 | `formats::binary`, `text` CPL | binary fixtures | M1 |
| TC-010, TC-011, TC-012 | `diff::algo::*`, `diff::closeness`, `formats::weights`, `text::compare` | P-ALIGN-*, P-CLOSE-*, align goldens | M1 |
| TC-014 | `diff::inline`, `text::compare::inline` | P-INLINE-*, inline unit tests | M1 |
| TC-016 | `text::quick` | P-TC-2 | M1 |
| TC-020, TC-021, TC-022 | `text::{rules, classify}`, `UnimportantItem` | §2.13.3 table, P-TC-7 | M1 |
| TC-024 (Ignore Unimportant) | `text::view`, `TextComparison::verdict` | P-TC-6 | M1 |
| TC-030, TC-037 (models) | `text::{Row, FilteredView, ViewItem}` | P-TC-4 | data |
| TC-032 | `TextComparison::thumbnail` | thumbnail unit tests | data |
| TC-035 | `ViewOptions::expanded`, `FilteredView::position_of` | view table | data |
| TC-040 | `text::view` | view table, P-TC-4 | M1 |
| TC-050 (section navigation) | `text::compare::nav` | unit tests | data |
| TP-003 | `report::text::{unified, context, normal}` | P-RPT-1, patch goldens | M1 |
| FF-001, FF-002, FF-003 | `formats::registry` | detection and shadowing tables, P-SHADOW-1 | M1 |
| FF-004, FF-040 | `formats/presets/*.toml`, `formats::presets`, `session::formats::FormatStore` | preset round trip, store table | M1 |
| FF-020 … FF-026 | `formats::{grammar, tokenize}` (D-41, D-58) | token goldens, P-TOK-* | M1 |
| FF-027 (b, c) | `text::classify`, `TextComparison::spans` | key table (a: GUI) | M1 |
| FF-030, FF-031 | `formats::convert`, `text::document` (Sorted) | `run_external` tests, Sorted unit test | M1 |
| VFS-020 | `vfs::compare` (CRC) | P-VFS-3 | M1 |
| CLI-001, CLI-002, CLI-003 (gui open subset) | `session::api::gui::{normalize_legacy_args, parse_open_args, infer_view_kind}` | legacy tables (§6.2.8), `cli_gui.rs` | M1 |
| CLI-004 | `core::exit`, `session::ApiError::exit_code` | §2.21 table end to end | M1 |
| RPT-002, RPT-003, RPT-004, RPT-010, RPT-011 | `report::{text, folder, meta, options}`, `Service::render_*` | report goldens, P-RPT-2/3 | M1 |
| SES-050 | `session::paths` | §2.22 table | M1 |
| SES-051 | `session::{formats, options}` | store and options tables | M1 |
| SES-053 | `vfs::AtomicWrite`, `session::atomic` | atomic write tests | M1 |
| SCR-001 (display vocabulary), SCR `criteria` | `report::DisplayMode`, `folder::{FolderDisplayFilter::parse, Criteria::parse_list}` | parse tests, §2.6.6 table | M1 |
| OPT: context lines, binary buffer size, content memory budget | `session::options::ProgramOptions` | options table | M1 |
| ARCH-GUI-002, ARCH-GUI-003 (enablers) | `TextComparison::{row, inline, thumbnail}`, `FilteredView::get`, `CompareObserver`, `FolderModel` | G5; wave 6 gate | data |

---

## Appendix A: Built-in presets

**Provenance rule (binding, 17-agent-workflow §5.1).** Keyword lists are written from public language
specifications and general programming knowledge. Nothing is copied from any other comparison product. Every
preset file starts with a `# Sources:` comment that names the specifications it was written from. The lists below
are the starting content; wave-2 review checks them against those specifications and confirms their
independence from other products.

### A.1 Complete example: `presets/02-c-cpp.toml`

```toml
# Sources: ISO/IEC 9899:2024 (C23) keywords; ISO/IEC 14882:2023 (C++23) keywords and alternative tokens;
# ECMA-334 (C#) keywords and common contextual keywords; the Objective-C language reference (@-directives and
# common types). Written for Hikasa.
schema_version = 1
id = "c-cpp"
name = "C, C++, C#, ObjC Source"
description = "C, C++, C# and Objective-C source files."
kind = "text"
mask = "*.c;*.cc;*.cpp;*.cxx;*.cs;*.h;*.hpp;*.hh;*.m;*.mm"

[encoding]
mode = "auto"

[text]
tab_size = 4

# Element settings by name. ElementIds follow first use in [[grammar]] (D-41), not this table's order.
[elements]
Number = { case_sensitive = false }   # 0xFF and 0xff are the same number

[[grammar]]
element = "Comment"
kind = "delimited"
from = "//"
stop_at_eol = true

[[grammar]]
element = "Comment"
kind = "delimited"
from = "/*"
to = "*/"

# Objective-C NSString literal (backslash escapes). C# verbatim strings (@"…" with "" doubling) are tokenised
# with the same rule; a trailing backslash in a C# verbatim string is a known imperfection ("simple by design",
# FF-026; Appendix C).
[[grammar]]
element = "String"
kind = "delimited"
from = '@"'
to = '"'
escape = '\'
stop_at_eol = true

[[grammar]]
element = "String"
kind = "delimited"
from = '"'
to = '"'
escape = '\'
stop_at_eol = true

[[grammar]]
element = "String"
kind = "delimited"
from = "'"
to = "'"
escape = '\'
stop_at_eol = true

[[grammar]]
element = "Directive"
kind = "basic"
text = '^\s*#\s*[A-Za-z_]\w*'
regex = true

[[grammar]]
element = "Keyword"
kind = "list"
match_case = true
tokens = [
  # C (C11–C23)
  "alignas", "alignof", "auto", "bool", "break", "case", "char", "const", "constexpr", "continue", "default",
  "do", "double", "else", "enum", "extern", "false", "float", "for", "goto", "if", "inline", "int", "long",
  "nullptr", "register", "restrict", "return", "short", "signed", "sizeof", "static", "static_assert",
  "struct", "switch", "thread_local", "true", "typedef", "typeof", "typeof_unqual", "union", "unsigned",
  "void", "volatile", "while", "_Alignas", "_Alignof", "_Atomic", "_BitInt", "_Bool", "_Complex",
  "_Decimal128", "_Decimal32", "_Decimal64", "_Generic", "_Imaginary", "_Noreturn", "_Static_assert",
  "_Thread_local",
  # C++
  "and", "and_eq", "asm", "bitand", "bitor", "catch", "char8_t", "char16_t", "char32_t", "class", "co_await",
  "co_return", "co_yield", "compl", "concept", "const_cast", "consteval", "constinit", "decltype", "delete",
  "dynamic_cast", "explicit", "export", "final", "friend", "import", "module", "mutable", "namespace", "new",
  "noexcept", "not", "not_eq", "operator", "or", "or_eq", "override", "private", "protected", "public",
  "reinterpret_cast", "requires", "static_cast", "template", "this", "throw", "try", "typeid", "typename",
  "using", "virtual", "wchar_t", "xor", "xor_eq",
  # C# (reserved and common contextual keywords)
  "abstract", "as", "async", "await", "base", "byte", "checked", "decimal", "delegate", "dynamic", "event",
  "fixed", "foreach", "get", "global", "implicit", "in", "init", "interface", "internal", "is", "lock",
  "nameof", "nint", "nuint", "object", "out", "params", "partial", "readonly", "record", "ref", "required",
  "sbyte", "sealed", "set", "stackalloc", "string", "uint", "ulong", "unchecked", "unsafe", "ushort", "value",
  "var", "when", "where", "with", "yield",
  # Objective-C
  "@autoreleasepool", "@catch", "@class", "@dynamic", "@encode", "@end", "@finally", "@implementation",
  "@import", "@interface", "@optional", "@package", "@private", "@property", "@protected", "@protocol",
  "@public", "@required", "@selector", "@synchronized", "@synthesize", "@throw", "@try", "BOOL", "Class", "IMP",
  "NO", "Nil", "SEL", "YES", "id", "instancetype", "nil", "self", "super",
]

[[grammar]]
element = "Identifier"
kind = "basic"
text = '[A-Za-z_]\w*'
regex = true

[[grammar]]
element = "Number"
kind = "basic"
text = '0[xX][0-9A-Fa-f\x27]+[uUlL]*|0[bB][01\x27]+[uUlL]*|\d[\d\x27]*(?:\.[\d\x27]*)?(?:[eE][+-]?\d+)?[uUlLfFdDmM]*|\.\d[\d\x27]*(?:[eE][+-]?\d+)?[fFdDmM]*'
regex = true

[[grammar]]
element = "Operator"
kind = "basic"
text = '[-+*/%=&|^!~<>?:.]+|[;,()\[\]{}@#$\\]'
regex = true
```

Element ids of this preset (D-41): 0 Everything Else, 1 Comment, 2 String, 3 Directive, 4 Keyword,
5 Identifier, 6 Number, 7 Operator. With the D-04 defaults this preset is **not** on the identity fast path,
because `Number` is case-insensitive.

### A.2 The other presets (order = `PRESETS` order; grammar items listed in order)

Unless a row says otherwise, every element has the default `ElementInfo { case_sensitive = true,
important = true }`, `word_chars` is empty, and the line weights are none.

| File | id / name | Mask | Grammar (in order), element settings and notes |
|---|---|---|---|
| 01-bash.toml | `bash` / Bash Scripts | `*.sh;*.bash;*.zsh` | EnvironmentVar (`\$\{[^}]*\}`, `\$[A-Za-z_]\w*`, `\$[0-9#?$!@*-]`), Comment (Delimited, regex `from` `(?:^\|(?<=\s))#`, to EOL: `#` at line start or after whitespace; look-behind → fancy-regex; to EOL so that trailing whitespace stays outside, D-13), String (`$'…'` esc `\`; `'…'` no escape, multi-line; `"…"` esc `\`, multi-line), Keyword list (§A.3), Identifier, Number, Operator; tab 4. Limitation: here-documents are not recognised, so an apostrophe in one (`Don't`) opens a `'…'` string that runs to the next apostrophe |
| 03-cobol.toml | `cobol` / COBOL Source | `*.cbl;*.cob;*.cpy` | LineNum (Columns 1–6), Comment (Basic regex `(?<=^.{6})[*/](?:.*\S)?`, i.e. `*` or `/` in column 7, up to its last non-blank char; fixed-length look-behind → fancy-regex, anchored so cheap), Identification (Columns 73–80, the identification area; `important = false`, so docs/10's "column 73–80 ignored" holds by default; column 73 is a field boundary, §2.10.2, so a literal or an identifier that reaches column 72 never swallows it and the blank padding before it is EE), String (`"…"` and `'…'`, doubled-delimiter escape, stop at EOL — a literal continued in the next line's indicator area is not modelled), Keyword (§A.3; `match_case = false`; element `case_sensitive = false`), Identifier (`[A-Za-z][A-Za-z0-9-]*`), Number; `word_chars = "-"` (D-58) |
| 04-pascal-forms.toml | `pascal-forms` / Object Pascal Forms | `*.dfm;*.fmx` | String (`'…'` doubled), Keyword (§A.3, case-insensitive; element `case_sensitive = false`), Identifier, Number, Operator; line weight `^\s*object` (regex, priority 4); tab 2, spaces |
| 05-pascal.toml | `pascal` / Object Pascal Source | `*.pas;*.dpr;*.dpk;*.inc` | Directive (`{$` … `}`), Comment (`//` EOL, `{` … `}`, `(*` … `*)`), String (`'…'` doubled), Keyword (§A.3, case-insensitive; element `case_sensitive = false`), Identifier (element `case_sensitive = false`: Pascal identifiers are case-insensitive), Number (`\$[0-9A-Fa-f]+` and decimals), Operator; weight `=\s*class\s*\(` priority 4 |
| 06-html.toml | `html` / HTML | `*.htm;*.html;*.xhtml;*.asp;*.aspx;*.ascx` | Comment (`<!--` … `-->`), String (attribute values: Basic regexes `(?<==\|=\s)"[^"<]*"` and `(?<==\|=\s)'[^'<]*'`, a quote right after `=` or `= ` up to the closing quote on the same line), Keyword (tag names: Basic regex `(?<=<\|</)[A-Za-z][\w.:-]*`; attribute names: Basic regex `[A-Za-z_:][\w.:-]*(?=\s*=)`; both fancy-regex; element `case_sensitive = false`), Entity (`&#?\w+;`), Operator (`</?`, `/?>`, `=`). Changed in the wave-2 review: the §A.3 word list matched words of running text everywhere ("The Table" → "The table" was unimportant); keywords are now found by position only. Limitations: valueless attributes (`disabled`) are EE; values that span lines or contain `<` are not strings |
| 07-java.toml | `java` / Java Source | `*.java;*.jav;*.kt` | as C (comments, strings, char), Annotation (`@\w+`), Keyword (Java plus Kotlin, §A.3; `non-sealed` is a non-word token tried individually), Identifier, Number, Operator |
| 08-javascript.toml | `javascript` / JavaScript Source | `*.js;*.mjs;*.ts;*.jsx;*.tsx` | as C (comments, `"…"`/`'…'` strings) + template strings (`` ` `` … `` ` `` multi-line, esc `\`), RegexLiteral (Basic regex `[(,=:\[!&\|?{};]\s*/(?![*/])(?:\\.\|\[(?:\\.\|[^\]\\])*\]\|[^/\\])+/[dgimsuvy]*`; the token deliberately includes the preceding punctuation, because context cannot be expressed without variable-length look-behind — "simple by design"), Keyword (ECMAScript plus TypeScript, §A.3), Identifier (`[A-Za-z_$][\w$]*`), Number, Operator; tab 2, spaces |
| 09-perl.toml | `perl` / Perl Scripts | `*.pl;*.pm` | POD (Delimited regex `^=[A-Za-z]\w*` … `^=cut\b.*`, element "Comment"), Comment (`#` EOL), String (`'…'`, `"…"` esc `\`), Variable (`[$@%][\w:]+`), Keyword (§A.3), Identifier, Number, Operator |
| 10-python.toml | `python` / Python Scripts | `*.py;*.pyw` | Indent (Basic regex `^[ \t]+(?=\S)`; look-ahead → fancy-regex, anchored so cheap; whitespace-only lines are not Indent), Comment (Basic regex `#(?:.*\S)?`: up to its last non-blank char, so that trailing blanks after a comment reach the Whitespace item; an end-of-line-terminated `#` item would end the line scan, §2.10.2), String (prefix-aware `from` regexes `[rRbBuUfF]{0,2}'''` / `"""` multi-line esc `\`, then `'…'` / `"…"` stop at EOL), Keyword (§A.3), Identifier, Number (`0[xob]…`, `_` separators, `j` suffix), Operator, Whitespace (Basic regex `[ \t]+$`: the doc's "trailing whitespace item"; element `important = false`, so trailing whitespace in Python is unimportant by default while `Indent` stays important; `--unimportant` cannot make it important, but a rules file `elements = { Whitespace = true }` can) |
| 11-ini-reg.toml | `ini-reg` / Registry Dumps / INI | `*.reg;*.ini` | Comment (`;` EOL; Basic regex `^\s*#.*`), Keyword: section headers (Basic regex `^\s*\[[^\]]*\]`), Keyword: value names (Basic regex `^\s*(?:"[^"]*"\|[^=;\[\s][^=]*?)(?=\s*=)`; look-ahead → fancy-regex, anchored), Keyword: value types (List `dword qword hex`, case-insensitive), String (`"…"` esc `\`), Operator (`=`) |
| 12-sql.toml | `sql` / SQL | `*.sql;*.dsql` | Comment (`--` EOL, `/*` … `*/`), String (`[nN]?'…'` doubled `''`), Keyword (§A.3, `match_case = false`, element `case_sensitive = false`), Identifier (`[@#]?[A-Za-z_][\w$#]*`, `"…"`, `[…]`), Number, Operator |
| 13-visual-basic.toml | `visual-basic` / Visual Basic Source | `*.bas;*.cls;*.frm;*.vb;*.vbs;*.ctl;*.vbp` | Comment (`'` EOL; Basic regex `(?i)\brem\b.*`), String (`"…"` doubled), Keyword (§A.3, case-insensitive; element `case_sensitive = false`), Identifier (element `case_sensitive = false`), Number (`&H[0-9A-F]+`, decimals), Operator |
| 14-xml.toml | `xml` / XML | `*.xml;*.xsd;*.xsl;*.xslt;*.svg;*.plist` | Comment (`<!--` … `-->`), CDATA (`<![CDATA[` … `]]>`, element "String"), Directive (`<?` … `?>`; also `<!` … `>` for declarations such as `<!DOCTYPE …>`), String (attribute values as in HTML: a quote right after `=` or `= `, on one line, without `<`), Entity (`&(?:#[0-9]+\|#x[0-9A-Fa-f]+\|[A-Za-z_][\w.-]*);`), Operator (`</?`, `/?>`, `=`). Limitation: an attribute value that spans lines (SVG path data) is not a string; the attributes after it still pair correctly |
| 15-rust.toml | `rust` / Rust Source | `*.rs` | Comment (`//` EOL; `/*` … `*/`, nesting approximated as non-nesting), Attribute (Basic regex `#!?\[[^\]]*\]`, single line), String (raw forms as three Delimited items `r"`…`"`, `r#"`…`"#`, `r##"`…`"##`, each also with a `b`/`c` prefix via regex `from`; then `[bc]?"` … `"` esc `\`, multi-line), Char (`b?'(?:\\.\|[^\\'])'`, listed before Lifetime), Lifetime (`'[A-Za-z_]\w*`, element "Identifier"), Keyword (§A.3), Identifier, Number (suffixes `u8…u128 i8…i128 usize isize f32 f64`, `_` separators), Operator |
| 16-sorted.toml | `sorted` / Sorted | (empty: manual only) | conversion `sorted`; no grammar |
| 17-csv.toml | `csv` / Comma Separated Values | `*.csv;*.tsv` (D-26) | kind `table`; `[table] delimiter = ","` (`*.tsv` is detected by the table engine in M2); compared as plain text in M1 (D-11) |
| 18-pictures.toml | `pictures` / Picture Files | `*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.ico;*.tif;*.tiff;*.webp;*.tga;*.pnm;*.qoi;*.exr;*.hdr;*.dds` | kind `picture`, mask only (binary comparison in M1) |
| 19-media.toml | `media` / Media Files | `*.mp3;*.mp4;*.m4a;*.m4b;*.m4p;*.m4v;*.flac;*.wav;*.ogg;*.opus` | kind `media`, mask only |
| 20-executables.toml | `executables` / Executables | `*.exe;*.dll;*.ocx;*.sys;*.drv;*.scr;*.bpl;*.so;*.dylib` | kind `version`, mask only |
| 21-registry-files.toml | `registry-files` / Registry Files | `*.reg` | kind `registry`, mask only; `Preset::enabled = false` (D-27) |

**Refinements accepted in the wave-2 review** (beyond the rows above; each is visible in the token goldens of
§6.2.4, which are the review artefact):

- **Number elements** are `case_sensitive = false` in c-cpp (A.1), java and javascript ("as C"), and also in
  pascal, python, sql and visual-basic (`0XFF` = `0xff`, `1E5` = `1e5`, `&HFF` = `&hff`).
- **bash**: `$'…'` strings are multi-line like the other quotes.
- **cobol**: strings stop at EOL; Comment and Identification as in the row (field boundary, §2.10.2).
- **pascal-forms**: String also `#\d+` (control-character literals); Number also `$` hex.
- **pascal**: String also `#(?:\$[0-9A-Fa-f]+|\d+)`; Identifier also `&name` (escaped reserved words);
  Number also `%` binary and `&` octal; the line weight `=\s*class\s*\(` is `match_case = false`.
- **java**: text blocks `"""` … `"""` (multi-line) before `"…"`; Number with `_` separators and suffixes.
- **javascript**: RegexLiteral as a linear-time `regex` form without look-ahead (the first body char may not be
  `*` or `/`): `[(,=:\[!&|?{};]\s*/(?:\\.|\[(?:\\.|[^\]\\])*\]|[^*/\\\[])(?:\\.|\[(?:\\.|[^\]\\])*\]|[^/\\\[])*/[dgimsuvy]*`;
  Number with BigInt `n`, `0o`, `0b`, `_`.
- **perl**: Variable (`\$#\{?[\w:]*\}?|[$@%][\w:]+`) comes before the `#` comment, so `$#array` is a variable.
- **python**: the soft keyword `_` is in the list; Identifier is Unicode (`[^\W\d]\w*`); Number also `.5`
  forms; Comment as in the row.
- **ini-reg**: the `#` comment is `^\s*#(?:.*\S)?` (D-13 spirit).
- **sql**: the `N'…'` string is a separate Delimited item with `match_case = false`; strings are multi-line;
  `"…"` and `[…]` identifiers stop at EOL.
- **visual-basic**: the REM comment is `\brem\b(?:.*\S)?` with `match_case = false` (instead of an inline
  `(?i)`, so the prefilter applies) and ends at its last non-blank char; Identifier with a type character
  (`[$%&!#@]?`); Number with `&O` octal and type suffixes.
- **html**, **xml**: as in the rows (positional keywords, strings after `=`, XML `<!…>` and Entity).
- **rust**: Char with `\u{…}`, `\x..` and one-char escapes; raw identifiers `r#name`; Unicode identifiers.

`# Sources:` lines of the other presets: bash — the GNU Bash reference manual (reserved words, builtins);
cobol — ISO/IEC 1989 (reserved words, reference format); pascal and pascal-forms — the Object Pascal language
guide (reserved words, directives, form-file text syntax); html — the WHATWG HTML Living Standard (tag and
attribute name syntax, attribute value quoting, comments, character references); java — the Java Language Specification (keywords, contextual keywords) and the Kotlin language
specification (hard, soft and modifier keywords); javascript — ECMA-262 (reserved words) and the TypeScript
handbook (type keywords); perl — perlsyn, perlfunc and perlop; python — the Python language reference (keywords,
soft keywords); ini-reg — INI conventions and the registry export text format; sql — ISO/IEC 9075 reserved words;
visual-basic — the Visual Basic language reference; xml — W3C XML 1.0; rust — the Rust Reference (strict,
reserved and weak keywords). Mask-only presets carry `# Sources: docs/10-file-formats.md §5`.

### A.3 Keyword lists (one List item per preset, element `Keyword`)

Case-sensitive unless marked; case-insensitive lists use `match_case = false` and an element with
`case_sensitive = false`.

- **bash**: reserved words `if then else elif fi case esac for select while until do done in function time
  coproc`; builtins `alias bg bind break builtin caller cd command compgen complete compopt continue declare
  dirs disown echo enable eval exec exit export false fc fg getopts hash help history jobs kill let local
  logout mapfile popd printf pushd pwd read readarray readonly return set shift shopt source suspend test
  times trap true type typeset ulimit umask unalias unset wait`.
- **c-cpp**: the list in A.1.
- **cobol** (case-insensitive, `word_chars = "-"`): `ACCEPT ACCESS ADD ADVANCING AFTER ALL ALSO ALTER AND ARE
  AREA ASCENDING ASSIGN AT AUTHOR BEFORE BLANK BLOCK BY CALL CANCEL CHARACTER CHARACTERS CLOSE COMP COMP-3
  COMPUTATIONAL COMPUTE CONFIGURATION CONTINUE COPY CORRESPONDING COUNT DATA DATE DELETE DELIMITED DELIMITER
  DEPENDING DESCENDING DISPLAY DIVIDE DIVISION DOWN DUPLICATES DYNAMIC ELSE END END-ADD END-CALL END-COMPUTE
  END-DELETE END-DIVIDE END-EVALUATE END-IF END-MULTIPLY END-PERFORM END-READ END-RETURN END-REWRITE END-SEARCH
  END-START END-STRING END-SUBTRACT END-UNSTRING END-WRITE ENVIRONMENT EQUAL ERROR EVALUATE EXCEPTION EXIT
  EXTEND FD FILE FILE-CONTROL FILLER FIRST FOR FROM FUNCTION GIVING GO GOBACK GREATER HIGH-VALUE HIGH-VALUES
  IDENTIFICATION IF IN INDEX INDEXED INITIALIZE INPUT INPUT-OUTPUT INSPECT INTO INVALID IS JUSTIFIED KEY
  LEADING LESS LINE LINES LINKAGE LOW-VALUE LOW-VALUES MERGE MODE MOVE MULTIPLY NEXT NOT NUMERIC OCCURS OF OFF
  ON OPEN OR ORGANIZATION OTHER OUTPUT OVERFLOW PERFORM PIC PICTURE PROCEDURE PROGRAM PROGRAM-ID RANDOM READ
  RECORD RECORDS REDEFINES RELATIVE RELEASE REMAINDER REPLACING RETURN REWRITE ROUNDED RUN SEARCH SECTION
  SELECT SENTENCE SEQUENTIAL SET SIGN SIZE SORT SPACE SPACES STANDARD START STATUS STOP STRING SUBTRACT
  TALLYING THAN THEN THROUGH THRU TIMES TO TRAILING UNSTRING UNTIL UP UPON USAGE USING VALUE VALUES VARYING
  WHEN WITH WORKING-STORAGE WRITE ZERO ZEROES ZEROS`.
- **pascal-forms** (case-insensitive): `object inherited inline end item true false`.
- **pascal** (case-insensitive): reserved words `and array as asm begin case class const constructor destructor
  dispinterface div do downto else end except exports file finalization finally for function goto if
  implementation in inherited initialization inline interface is label library mod nil not object of operator
  or out packed procedure program property raise record repeat resourcestring set shl shr string then
  threadvar to try type unit until uses var while with xor`; directives `absolute abstract assembler automated
  cdecl default deprecated dynamic export external far forward helper implements index message name near
  nodefault overload override platform private protected public published read readonly reference register
  reintroduce requires resident safecall sealed static stdcall stored strict unsafe varargs virtual write
  writeonly`.
- **html** (case-insensitive; *superseded in the wave-2 review*: the preset finds tag and attribute names by
  position instead, Appendix A.2, so this list is no longer shipped; it is kept as the reference vocabulary):
  tags `html head body title meta link script style div span p a img ul ol li
  table tr td th thead tbody tfoot caption col colgroup form input button select option optgroup textarea
  label fieldset legend datalist output progress meter h1 h2 h3 h4 h5 h6 br hr nav header footer section
  article aside main figure figcaption iframe pre code em strong b i u s small sub sup blockquote q cite abbr
  time mark details summary dialog picture source track video audio canvas svg template slot noscript base
  area map object embed dl dt dd`; attributes `id class style href src alt title type name value rel lang
  charset content width height action method target for placeholder disabled checked selected readonly
  required hidden tabindex role autocomplete autofocus multiple pattern min max step colspan rowspan`.
- **java**: Java `abstract assert boolean break byte case catch char class const continue default do double
  else enum exports extends final finally float for goto if implements import instanceof int interface long
  module native new non-sealed open opens package permits private protected provides public record requires
  return sealed short static strictfp super switch synchronized this throw throws to transient transitive try
  uses var void volatile when while with yield true false null`; Kotlin `actual annotation as by companion
  constructor crossinline data dynamic expect external field fun get infix init inner internal is lateinit
  noinline object operator out override reified set suspend tailrec typealias val value vararg where`.
- **javascript**: ECMAScript `await break case catch class const continue debugger default delete do else enum
  export extends false finally for function if import in instanceof let new null return static super switch
  this throw true try typeof undefined var void while with yield async of get set`; TypeScript `abstract
  accessor any as asserts bigint boolean declare implements infer interface is keyof module namespace never
  number object override private protected public readonly require satisfies string symbol type unique
  unknown`.
- **perl**: `if elsif else unless while until for foreach do last next redo return goto my our local state sub
  package use no require BEGIN END INIT CHECK UNITCHECK and or not xor eq ne lt gt le ge cmp print printf say
  die warn eval defined undef ref bless shift unshift push pop splice keys values each exists delete scalar
  wantarray open close chomp chop split join map grep sort reverse sprintf length substr index lc uc
  __PACKAGE__ __FILE__ __LINE__ __END__ __DATA__`.
- **python**: keywords `False None True and as assert async await break class continue def del elif else
  except finally for from global if import in is lambda nonlocal not or pass raise return try while with
  yield`; soft keywords `match case type _`.
- **rust**: strict `as async await break const continue crate dyn else enum extern false fn for if impl in let
  loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where
  while`; reserved `abstract become box do final gen macro override priv try typeof unsized virtual yield`;
  weak `union macro_rules`.
- **sql** (case-insensitive): `ADD ALL ALTER AND ANY AS ASC BEGIN BETWEEN BIGINT BOOLEAN BY CASCADE CASE CAST
  CHAR CHECK CLOSE COLUMN COMMIT CONSTRAINT CREATE CROSS CURSOR DATABASE DATE DECIMAL DECLARE DEFAULT DELETE
  DESC DISTINCT DROP ELSE END ESCAPE EXCEPT EXEC EXECUTE EXISTS FETCH FLOAT FOREIGN FROM FULL FUNCTION GRANT
  GROUP HAVING IF IN INDEX INNER INSERT INT INTEGER INTERSECT INTO IS JOIN KEY LEFT LIKE LIMIT MERGE NATURAL
  NOT NULL NUMERIC NVARCHAR OFFSET ON OPEN OR ORDER OUTER OVER PARTITION PRIMARY PROCEDURE REAL RECURSIVE
  REFERENCES RETURN RETURNS REVOKE RIGHT ROLLBACK ROW ROWS SCHEMA SELECT SET SMALLINT TABLE TEXT THEN
  TIMESTAMP TOP TRANSACTION TRIGGER TRUNCATE UNION UNIQUE UPDATE USING VALUES VARCHAR VIEW WHEN WHERE WHILE
  WITH`.
- **visual-basic** (case-insensitive): `AddHandler AddressOf Alias And AndAlso As Boolean ByRef Byte ByVal Call
  Case Catch CBool CByte CChar CDate CDbl CDec Char CInt Class CLng CObj Const Continue CSByte CShort CSng
  CStr CType CUInt CULng CUShort Date Decimal Declare Default Delegate Dim DirectCast Do Double Each Else
  ElseIf End EndIf Enum Erase Error Event Exit False Finally For Friend Function Get GetType Global GoSub GoTo
  Handles If Implements Imports In Inherits Integer Interface Is IsNot Let Lib Like Long Loop Me Mod Module
  MustInherit MustOverride MyBase MyClass Namespace Narrowing New Next Not Nothing NotInheritable
  NotOverridable Object Of On Operator Option Optional Or OrElse Overloads Overridable Overrides ParamArray
  Partial Private Property Protected Public RaiseEvent ReadOnly ReDim RemoveHandler Resume Return SByte
  Select Set Shadows Shared Short Single Static Step Stop String Structure Sub SyncLock Then Throw To True Try
  TryCast TypeOf UInteger ULong UShort Using Variant Wend When While Widening With WithEvents WriteOnly Xor`.
- **ini-reg**: value types `dword qword hex` (case-insensitive); everything else is regex-based (A.2).
- **xml**, **sorted**, **csv** and the mask-only presets: no keyword list.

Implementation notes for preset authors (normative):

- A grammar item whose regex needs look-around compiles with fancy-regex (§2.11). Presets should prefer
  linear-time `regex` forms, because 1M-line throughput (§6.4) depends on it.
- Item order matters (first match wins). Comments and strings come before keywords and identifiers. Character
  and lifetime rules are ordered as listed.
- Each preset ships with at least two token goldens (§6.2.4), and those goldens are the review artefact for
  grammar changes.

---

## Appendix B: Invariant catalogue

| ID | Invariant | Checked by |
|---|---|---|
| INV-SPEC-1..3 | `LocalPath` is absolute and normalised (no empty or `.` components; no trailing separator except the root); canonical forms never contain a password | `LocalPath::new`, P-SPEC-1/2, unit tests |
| INV-SPEC-4 | `parse(canonical(s)) == s.without_secrets()` | P-SPEC-1 |
| INV-RELPATH-1 | no empty, `.` or `..` components; `/` separators | `RelPath::parse`, unit tests |
| INV-MASK-1 | matching is total and costs O(\|P\|·\|N\|) | P-MASK-1 (no panics), bench |
| INV-MASK-2 | list parse and print round-trip | P-MASK-2 |
| INV-CANCEL-1/2 | cancelling a parent cancels its children; cancelling a child never cancels the parent | unit tests |
| INV-TIME-1 | `nanos < 1e9` | constructor, P-TIME-1 |
| INV-NAME-1 | `FileName::raw` is `Some` iff the name is lossy | unit tests |
| INV-VFS-1 | listing order is unspecified; consumers sort | shuffled `MemoryVfs` in P-FOLDER-2 |
| INV-VFS-2 | mmap (every thread count) = stream = naive comparison (including the offset); mapped CRC = streamed CRC | P-VFS-1/2, `p_vfs_2_multi_chunk` |
| INV-VFS-3 | atomic writes: nothing is visible before `commit`; drop discards | unit tests |
| INV-INTERN-1 | ids are dense, in first-seen order (left then right) | P-INTERN-1 |
| INV-DENSE-1 | every imara-diff call receives tokens remapped by `DenseRemap` to `0..k`, with `num_tokens = k` and every token `< k` (debug assertion) | P-ALIGN-9, `align_pathological` |
| INV-ALIGN-1 | runs cover `[0,n)` and `[0,m)` contiguously, in order | `check_invariants`, P-ALIGN-1 |
| INV-ALIGN-2 | Equal runs: equal lengths, equal keys pairwise | P-ALIGN-1 |
| INV-ALIGN-3 | Changed runs: equal lengths, different keys pairwise | P-ALIGN-1 |
| INV-ALIGN-4 | Delete has an empty right range, Insert an empty left range; no run is empty | P-ALIGN-1 |
| INV-ALIGN-5 | canonical form: no two adjacent runs of the same kind; Delete before Insert inside a gap | P-ALIGN-1 |
| INV-ALIGN-6 | identical inputs give exactly one Equal run | P-ALIGN-3 |
| INV-ALIGN-7 | never-align gives no Changed runs; Unaligned pairs only `(k, k)` | P-ALIGN-5 |
| INV-CLOSE-1..3 | closeness pairs are monotone, above the threshold, and optimal | P-CLOSE-* |
| INV-INLINE-1..4 | inline ranges are sorted, disjoint, non-empty, on boundaries; the common text is equal; `a == b` gives no ranges; determinism | P-INLINE-* |
| INV-TOK-1..5 | tokens are non-empty, contiguous, cover the line, are merged; context-free per line; parallel = sequential | P-TOK-* |
| INV-FMT-1 | `parse_format(to_toml(f)) == f` | preset round trip |
| INV-FMT-2 | `ElementId`s are dense: 0 is EE, then first use in `grammar`, then unused declared elements by name (D-41); independent of TOML key order | unit tests |
| INV-RULES-1 | `TextRules::from_toml(to_toml(r)) == r` for every value | rules round-trip test |
| INV-RULES-2 | `FolderRules::from_toml(to_toml(r)) == r` for every value, including `timestamps.enabled = false`, offsets and filters | P-RULES-2 |
| INV-ENC-1 | decoding is total; UTF round trips | P-ENC-1/2 |
| INV-KEY-1 | key encoding is prefix-free, hence injective over atom sequences (*wave 3:* text bytes < 0xF8, self-delimiting markers ≥ 0xF8) | unit test with crafted collisions |
| INV-KEY-2 | trivial ⇔ the key has no TEXT or TOKEN atom (identity path: ⇔ the line is empty) (*wave 3:* ⇔ the key has no text byte) | key table |
| INV-KEY-3 | the identity fast path and the general path induce the same equivalence on lines, hence identical results (D-53) (*wave 3:* for every grammar: under identity rules the general key equals the identity key byte for byte) | P-TC-7 (multi-line grammars, never-matching pattern), unit test `all_important_keys_are_the_raw_line_in_every_state` |
| INV-DOC-1 | lines end only at CR, LF or CRLF; lines + non-artificial endings reproduce the decoded text | P-TC-1 helper |
| INV-DOC-2 | at most one `None` ending, on the last line | unit tests |
| INV-ROW-1 | `Same` ⇔ importance `None` | `check_invariants` |
| INV-TC-1..8 | rows correspond 1:1 to alignment rows; line coverage and order on both sides; sections are maximal, disjoint and cover exactly the non-Same rows; statistics identities; the verdict rules; quick consistency; parallel = sequential; importance follows the rules (*wave 3:* INV-TC-8 — Equal pairs are Same iff raw-equal, Changed pairs are Important iff not raw-equal, raw equality including the artificial-break flag, §2.17.1) | `TextComparison::check_invariants`, P-TC-* |
| INV-VIEW-1..3 | items cover all rows in order; no adjacent gaps; Context merges like unified hunks | P-TC-4 |
| INV-FOLDER-1 | a well-formed tree: parent and child links agree; the root has no parent | `FolderComparison::check_invariants` |
| INV-FOLDER-2 | kinds never mix inside a node (D-20) | P-FOLDER-1 |
| INV-FOLDER-3 | children form one contiguous id block in canonical order (§2.5.4) | P-FOLDER-2 |
| INV-FOLDER-4 | an orphan node has exactly one present side | P-FOLDER-1 |
| INV-FOLDER-5 | every stored counter (`Aggregates`, both iu values) and every `SideTotals` equals a recount, after every applied batch and at the end; statistics equal a recount | P-FOLDER-1, P-FOLDER-6 |
| INV-FOLDER-6 | a folder's state is `own_category(F, false)` of its counters (§2.6.4) | P-FOLDER-1 |
| INV-FOLDER-7 | `filtered` flags agree with re-evaluating the filters; name-only entries only occur on filtered sides; an entry without metadata is filtered iff the name masks exclude its name (file masks) and is never name-only (§1.6.8 item 3) | P-FOLDER-5 |
| INV-FOLDER-8 | `content.is_some()` ⇒ kind File, both sides present and stat'ed, not filtered | P-FOLDER-1 |
| INV-FOLDER-9 | `visible_rows` equals the recursive "visible and every ancestor listed" definition of §2.7 | P-FOLDER-7 |
| INV-FOLDER-10 | ignore-unimportant monotonicity: a difference category under `iu = true` is a difference under `iu = false`; Unknown/Error do not depend on `iu` | P-FOLDER-8 |
| INV-EVT-1..3 | `Started` first and `Finished` last; `FolderListed{F}` precedes every event naming a child of F; every event is sent after its mutation is visible, so re-reading the named nodes, the children of listed folders and the ancestors of both (`TreeRead::ancestors`; amended in §1.6.8 item 21: ancestors' counters change without being named) at each event ends with exactly the final tree | P-FOLDER-4 |
| INV-RPT-1 | reports are byte-identical for identical inputs | P-RPT-2 |
| INV-RPT-2 | a patch applied to the left file reproduces the right file byte for byte (lone CR and artificial breaks as LF with `patch_lossy_eol`; other encodings warned `patch_encoding`; with Ignore Unimportant, the hunks without an important row are left out) | P-RPT-1, `tests/apply.rs` (wave 4) |
| INV-CLI-1..6 | the CLI invariants of §1.9.2 | §6.3 |
| INV-IPC-1..4 | one immediate response per request; at most one `ViewClosed` per wait (*wave-4 review:* exactly one while the connection lives — a dropped waiter sends `abandoned`, a shutdown sends `shutdown`); frames ≤ `MAX_FRAME_LEN` and ≤ `MAX_FRAME_VALUES` JSON values; an unknown `type` or a body that does not decode is answered with the request's own id (D-46) | IPC tests, `ipc_review.rs`, `ipc_memory.rs` |
| INV-IPC-5 | a client never sends anything to an endpoint whose socket folder fails the §2.23 verification (*wave-4 review*) | `ipc_review.rs` |
| INV-SES-1 | `FolderCompareResponse` JSON, reports and exit code use the same effective ignore-unimportant (*wave-4 review*) | `review.rs` |
| INV-BUDGET-1 | the bytes held by content tests never exceed the `ByteBudget` capacity | `ByteBudget` tests |

---

## Appendix C: Open questions for the maintainer

Each item carries a default decision, which is binding for M1 unless the maintainer overrides it.

1. **Per-pane formats (D-15).** M1 uses one effective format per comparison. M2 needs a rule for pairing keys
   from different grammars. Proposal: build keys from element *names* rather than ids, and treat a missing
   grammar as plain text.
2. **Windows named-pipe security.** M1 relies on a per-user pipe name and default pipe security. P1 should add
   an explicit owner-only DACL through `interprocess`'s security-descriptor support.
3. **NFC matching in masks (D-07).** Canonically equivalent names match. Confirm.
4. **Default text importance (D-04).** Defaults are exact (only line endings, encoding and BOM are ignored);
   ignoring whitespace is the opt-in `--unimportant whitespace`, and presets may ship unimportant elements
   (Python trailing whitespace, COBOL columns 73–80). Confirm.
5. **Option naming.** `--rules-file` is used everywhere (`--rules` stays an alias for `text compare` and
   `folder compare`, because `quick --rules` is a method flag). Needs the docs/12 update of §0.9.
6. **Half-hour time zones (D-28).** FC-021 says "multiples of 1 h". Should 30- and 45-minute offsets count too?
   Default: no.
7. **Win32 path normalisation.** Should trailing dots and spaces in the last component be stripped, as Win32
   does? Default: no in M1 (Linux first); revisit with Windows CI.
8. **Reproducible CLI reports.** Should `SOURCE_DATE_EPOCH` (or a hidden flag) set `ReportMeta::generated_at`?
   Default: no. Goldens drop the `Generated:` line instead.
9. **Content-test memory (D-25).** Per-file rules limit 128 MiB, shared budget 1 GiB, and the new
   `options.toml` key `[tweaks.file_operations] content_memory_budget_mib` (added to docs/15 in wave 4).
   Confirm the numbers and the key.
10. **C# verbatim strings in the shared C-family preset.** Options: split C# into its own preset, or accept the
    imperfection. Default: accept it in M1.
11. **Clipboard specs.** No clipboard crate is approved, so `clipboard://` resolves only when an adapter injects
    a `Blob`, and `gui open clipboard://` is rejected (D-49). Adding `arboard` (listed in docs/02) is an M2
    dependency decision.
12. **Folder `display-context` (reports).** Mapped to "Differences + Compare Files and Folder Structure"
    (`DisplayMode::folder_filter`). Confirm.
13. **ARCH-061 comparison priority (P0).** Needs an OS API (`libc` setpriority / `SetThreadPriority`), and no
    such crate is approved. Default: gap in M1; the service pool builder is the single place where a priority
    would be applied. Approve `libc` or `thread-priority`, or defer to M2.
14. **Owner and group names (FC-024, FC-040).** M1 shows numeric uid/gid. Names need `getpwuid_r`/`getgrgid_r`
    (libc) or parsing `/etc/passwd` and `/etc/group`. Which approach?
15. **Graceful Ctrl-C.** Without a signal-handling crate the CLI terminates on SIGINT; outputs are atomic.
    Default: accept in M1.
16. **GUI toolkit dependency (D-56).** Dioxus is not in `[workspace.dependencies]`; wave 6 waits for approval.
17. **`gui open --no-launch` without `--wait` when no GUI runs (D-23).** CLI-004 can be read either way (100 or
    102). Default: 100 ("not running"), because CLI-004 defines 102 as "CLI helper unable to wait for the GUI";
    with `--wait` the code is 102. Confirm.
18. **Suppress Filters (D-44).** Suppress Filters reveals file-filtered items in teal and the display filter
    still applies to everything else. Confirm.
19. **Name-only filtered entries (D-45).** Items hidden by name masks are not stat'ed, so under Suppress
    Filters they show name and kind without size or date (the filtered side of an FC-054 pair is stat'ed).
    Confirm the trade-off.
20. **Encoding fallback preference (D-42).** chardetng 1.0 exposes `find_score` only behind a test-only feature,
    so a format's fallback encoding applies only to content without non-ASCII bytes. Revisit if a public API
    appears.
21. **`--criteria filename-case` (D-57).** A Hikasa extension of the script vocabulary for FC-022. Confirm the
    word.
22. **DOS code pages (FF-032).** `encoding_rs` covers IBM866 but not the other DOS code pages FF-032 lists.
    Default: unsupported labels are `FormatError::UnknownEncoding` in M1; in-house single-byte tables are a P1
    item.
