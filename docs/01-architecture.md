# Architecture

## 0. Three layers: core, CLI, GUI

The product is organised in three strictly layered groups of crates:

```
┌──────────────────────────────────────────────────────────────────┐
│ gui/      hikasa-gui      desktop application (Dioxus)           │  presents
│                           hosts the IPC endpoint "open a view"   │
├──────────────────────────────────────────────────────────────────┤
│ cli/      hikasa-cli      `hikasa` binary: every function of the │  exposes
│                           product as a subcommand, headless,     │
│                           machine-readable output, exit codes;   │
│                           `hikasa gui …` connects to the GUI     │
├──────────────────────────────────────────────────────────────────┤
│ core/     hikasa-core, -vfs, -diff, -formats, -folder, -text,    │  computes
│           -table, -hex, -picture, -metadata, -report, -script,   │
│           -session   — libraries only, no UI, no process I/O     │
└──────────────────────────────────────────────────────────────────┘
```

- **ARCH-L1** (P0): **Core** crates are libraries. They contain all comparison,
  merge, sync, conversion, report, script and settings logic. They never link a UI
  toolkit, never print to a terminal, never read environment/arguments. They are the
  only place where product behaviour is implemented.
- **ARCH-L2** (P0): **CLI-first rule.** Every tool and every function of the product
  is available from the `hikasa` command line, headless, before (or at the same time
  as) it is available in the GUI. A feature without a CLI subcommand is not done
  (see `17-agent-workflow.md`). The CLI is a thin adapter: argument parsing, output
  formatting (human text, `--json`), exit codes, progress on stderr. It calls core
  directly; it does not need the GUI to run.
- **ARCH-L3** (P0): The **GUI** is a second thin adapter over the same core crates.
  It must not reimplement anything that exists in core, and it must not call the CLI
  binary to do work. GUI and CLI share the session/settings files, so a session
  created from the CLI opens unchanged in the GUI and vice versa.
- **ARCH-L4** (P0): **CLI → GUI connection.** The GUI process listens on a local IPC
  endpoint (Unix domain socket / Windows named pipe, per user, authenticated by file
  permissions). `hikasa gui <subcommand>` (and the legacy positional forms,
  `12-cli-and-scripting.md`) send a request to open a view (compare, merge, sync, edit,
  patch, session, workspace), optionally wait for the view to close, and relay its
  result as the exit code. If no GUI is running, the CLI starts it (unless
  `--no-launch`). The IPC protocol is JSON over length-prefixed frames and is
  documented and versioned in `hikasa-session::ipc` so that other tools (editors,
  version-control front-ends) can use it.
- **ARCH-L5** (P1): The GUI can also *embed a terminal-less CLI pane* ("Command"
  panel) that runs any `hikasa` subcommand in-process through the same core APIs, so
  users can learn the CLI from the GUI (every GUI action can show its CLI equivalent).

## 1. Principles

- **ARCH-001** (P0): The product is a Cargo workspace of small crates. Every crate
  except the GUI crate is UI-agnostic and compiles headless (used by the CLI, by
  scripts and by tests).
- **ARCH-002** (P0): Comparison engines are *pure*: given two inputs and rules they
  produce an immutable result model. They do no I/O themselves; I/O goes through the
  VFS layer. This keeps them testable and reusable across views and reports.
- **ARCH-003** (P0): All long work (scanning, hashing, diffing, transfers) runs off
  the UI thread on a work pool, is cancellable via a `CancellationToken`, and reports
  progress through a channel. The UI only receives *messages*.
- **ARCH-004** (P0): Results are delivered incrementally where the algorithm permits
  (folder scan, standard alignment), so the user sees partial output early.
- **ARCH-005** (P0): No `unsafe` in application crates; `unsafe` is permitted only in
  a dedicated, audited crate (e.g. memory-mapped I/O wrappers) and must be justified
  in a comment.
- **ARCH-006** (P1): Every user-visible string lives in a localization table (English
  is the source language). No hard-coded UI strings in views.
- **ARCH-007** (P0): Settings, sessions, formats and profiles are plain, documented,
  human-readable files (TOML) stored in the per-user config directory. A *portable
  mode* stores them next to the executable.

## 2. Workspace layout

```
Hikasa/
├── Cargo.toml                 # workspace, version 0.0.0
├── core/                      # layer 1: libraries only
│   ├── hikasa-core/           # shared types: Side, Spec, errors, cancellation, progress, settings schema, localization
│   ├── hikasa-vfs/            # virtual file system: local, archive, snapshot, remote back-ends, clipboard, http
│   ├── hikasa-diff/           # alignment engines (standard, Myers, patience, closeness, column), importance classification
│   ├── hikasa-formats/        # file format model, grammar engine (tokenizer), conversions, built-in format presets
│   ├── hikasa-folder/         # folder scan/compare/merge/sync model, file operations, filters, snapshots
│   ├── hikasa-text/           # text document model (rope), text compare/merge/edit/patch session logic
│   ├── hikasa-table/          # tabular parsing (delimited/fixed/spreadsheet), table compare
│   ├── hikasa-hex/            # binary document model, hex compare
│   ├── hikasa-picture/        # image loading, pixel compare, metadata compare
│   ├── hikasa-metadata/       # media tags, executable version info, registry files (tree-of-values compare)
│   ├── hikasa-report/         # report generation (text, HTML, XML, patch, statistics, print model)
│   ├── hikasa-script/         # script language parser + interpreter (drives hikasa-folder headless)
│   └── hikasa-session/        # session/workspace/profile/options persistence, import/export packages, IPC protocol
├── cli/                       # layer 2
│   └── hikasa-cli/            # `hikasa` binary: every function as a subcommand; `hikasa gui …` talks to the GUI
├── gui/                       # layer 3
│   └── hikasa-gui/            # desktop application (Dioxus): windows, tabs, views, dialogs, IPC server
└── docs/
```

Dependency direction (arrows point to dependencies):

```
gui/hikasa-gui ──► hikasa-session ──► hikasa-folder ──► hikasa-vfs ──► hikasa-core
cli/hikasa-cli ──┘      │                  │               ▲
                        ├──► hikasa-text ──┼──► hikasa-diff ┘
                        ├──► hikasa-table ─┤
                        ├──► hikasa-hex ───┤
                        ├──► hikasa-picture┤
                        ├──► hikasa-metadata
                        ├──► hikasa-formats
                        ├──► hikasa-report
                        └──► hikasa-script
```

- **ARCH-010** (P0): `hikasa-core` has no dependency on any other workspace crate.
- **ARCH-011** (P0): Engine crates (`diff`, `text`, `table`, `hex`, `picture`,
  `metadata`) must not depend on `hikasa-gui`, `hikasa-cli` or `hikasa-session`.
- **ARCH-012** (P0): `hikasa-gui` and `hikasa-cli` contain no comparison logic; they
  map core results to widgets/output and user input to core calls. CI enforces that no
  `core/` crate depends on `cli/` or `gui/` (`cargo deny` bans + a workspace test).
- **ARCH-013** (P0): Every public operation of `hikasa-session` (open session, run
  comparison, apply operation, save, report, …) is exposed as a **service API**
  (`hikasa_session::api`) consumed identically by the CLI and by the GUI. New
  behaviour is added to the API first, then wired to both adapters.

## 3. Key abstractions

### 3.1 `Spec` (hikasa-core)

A `Spec` identifies a file or folder on any back-end:

```
enum Spec {
    Local(PathBuf),                                  // /home/u/x, C:\x, \\server\share\x, ~/x
    ArchiveMember { archive: Box<Spec>, path: String }, // x.zip/inner/file
    Snapshot { file: PathBuf, path: String },        // snapshot.hksnap/inner
    Remote { profile: ProfileRef, path: String },    // ftp://u@h/p, sftp://, ftps://, dav(s)://, s3://bucket/key, profile:Name?path
    Http(Url),                                       // single files only
    Clipboard,                                       // clipboard://
    Stdin,                                           // "-"
}
```

- **ARCH-020** (P0): Specs round-trip to a canonical string form used in sessions,
  on the command line and in scripts (`11-vfs-remote-archives.md` §2).
- **ARCH-021** (P0): Archive members nest (`a.zip/b.tar.gz/c.txt`).

### 3.2 VFS (hikasa-vfs)

```
trait Vfs {
    fn stat(&self, spec:&Spec) -> Result<Entry>;
    fn list(&self, dir:&Spec, cancel:&Cancel) -> Result<Box<dyn Iterator<Item=Result<Entry>>>>;
    fn open_read(&self, spec:&Spec) -> Result<Box<dyn ReadSeek>>;   // streaming
    fn open_write(&self, spec:&Spec, opts:WriteOpts) -> Result<Box<dyn Write>>;
    fn mkdir / remove / rename / copy_within / set_times / set_attrs / set_mode ...
    fn capabilities(&self) -> Caps; // timestamps writable? attrs? unix mode? recycle bin? locks? crc provided?
}
struct Entry { name, kind: File|Dir|Symlink, size, mtime, ctime?, attrs: DosAttrs|UnixMode, owner, group, link_target, crc32: Option<u32>, version: Option<VersionInfo> }
```

- **ARCH-030** (P0): Every back-end exposes capabilities; UI and operations degrade
  gracefully (e.g. "touch" greyed out when timestamps are read-only).
- **ARCH-031** (P0): Copy between back-ends is implemented generically (stream from
  source `open_read` to target `open_write`, then apply times/attrs if supported),
  with a fast path for local→local.

### 3.3 Session model (hikasa-session)

```
struct Session<T: SessionKind> { id, name: Option<String>, folder: Option<String>, locked: bool,
                                  specs: T::Specs, rules: T::Rules, view_state: T::ViewState,
                                  description: String, auto_saved_at: Option<Time> }
```

- Each session type defines `Specs`, `Rules`, `ViewState` as serde-serializable structs.
- **ARCH-040** (P0): "Defaults" sessions (one per type) are the template for new
  sessions; editing them changes defaults for future sessions only.
- **ARCH-041** (P0): Session settings changes have a *scope*: this view only /
  update defaults / (for child sessions) these files within parent / all files within
  parent.

### 3.4 Comparison result models

Each engine returns an immutable result that the view renders:

- Folder: a tree of `Node { left: Option<Entry>, right: Option<Entry>, center…, state: CompareState, children }` with aggregate state per folder.
- Text: a `Alignment { rows: Vec<Row> }` where `Row { left: Option<LineIdx>, right: Option<LineIdx>, center…, kind: Same|Diff|LeftOrphan|RightOrphan|…, importance, section_id, char_ranges }`.
- Table: alignment of rows plus per-cell comparison.
- Hex: alignment of byte ranges.
- Picture: per-pixel difference map plus metadata tree compare.
- Metadata: a tree of key/value pairs compared by name.

- **ARCH-050** (P0): Result models carry stable identifiers so edits can be applied
  incrementally (re-diff only affected sections) and so undo/redo can refer to them.

### 3.5 Jobs, progress, cancellation (hikasa-core)

```
struct Job { id, kind, title, progress: watch::Receiver<Progress>, cancel: CancellationToken }
enum Progress { Indeterminate, Fraction{done,total}, Items{done,total,current:String}, Done(Result<()>) }
```

- **ARCH-060** (P0): A global job registry drives the status bar, the per-view progress
  panel and the Log panel. Each file operation logs a summary line and optional
  per-file details (see `15-options-reference.md` Log Panel).
- **ARCH-061** (P0): Comparison priority (Normal / Below normal / Idle) is a program
  option applied to the work pool.

### 3.6 Commands (hikasa-gui, catalogue shared with the CLI)

- **ARCH-070** (P0): Every user action is a `Command { id, category, caption, hint,
  default_shortcut, secondary_shortcuts, menu: bool, toolbar: bool, enabled_when }`
  registered in a per-view-type catalogue (`reference/commands.md`). Menus, toolbars,
  context menus and the "Toolbars, Shortcuts, Menus" options page are all generated
  from this catalogue; shortcuts are user-configurable per view type.

## 4. Threading and runtime

- **ARCH-080** (P0): CPU-bound work (alignment, hashing, pixel diff) uses a `rayon`
  pool sized to the machine; I/O-bound work (remote transfers, watch events) runs on a
  `tokio` multi-thread runtime. Engines expose synchronous APIs; the session layer
  decides where to run them.
- **ARCH-081** (P0): Large local files are memory-mapped for hashing, binary compare
  and hex views; text views load into a rope with a configurable "characters per line"
  limit and lazy syntax tokenization.
- **ARCH-082** (P1): The folder scanner streams directory entries with a bounded
  channel; subfolders scan in the background ("auto-scan") with a configurable
  concurrency, and the view colours folders as their aggregate state becomes known.
- **ARCH-083** (P1): File watching (`notify`) powers "check for files changed on disk"
  and "automatic refresh" for local back-ends; remote back-ends poll on a timer.

## 5. Single instance and IPC

- **ARCH-090** (P0): Only one GUI instance runs per user unless `--solo` is given. A
  second GUI launch forwards its command line to the running instance over the IPC
  endpoint and exits.
- **ARCH-091** (P0): IPC protocol (`hikasa_session::ipc`): JSON messages, length-
  prefixed, over a per-user local socket (`$XDG_RUNTIME_DIR/hikasa.sock`,
  `\\.\pipe\hikasa-<user>`). Requests: `Hello{version}`, `OpenView{kind, specs,
  options, wait}`, `LoadSession{name}`, `LoadWorkspace{name}`, `RunScript{path,args}`,
  `ListViews`, `CloseView{id}`, `Ping`, `Shutdown`. Responses carry a view id, and
  when `wait` was requested a final `ViewClosed{id, result, exit_code}`. Unknown
  fields are ignored; the version is negotiated in `Hello`.
- **ARCH-092** (P0): `hikasa gui …` (CLI) implements the client side; `hikasa-gui`
  the server side. The CLI "wait" mode stays alive until the view closes and returns
  its exit code, which is what version-control systems need.

## 6. Data flow example — Folder Compare with child Text Compare

1. User enters two specs; the session layer resolves back-ends via VFS.
2. A scan job lists both base folders, aligns entries by name (plus alignment
   overrides, extension-insensitive and Unicode-normalization options), applies
   name/other filters and quick tests (size, timestamp, attributes), emitting tree
   updates.
3. If rules require content tests, content jobs run per file pair (CRC, binary or
   rules-based through the file-format engine) and update node states.
4. User double-clicks a pair: a Text Compare session is created with the pair's
   specs, the parent's enabled file formats and parent-scoped rule overrides; the
   result (same / unimportant / important differences) is sent back to the parent
   node when the child finishes loading or saves.

## 7. Error handling

- **ARCH-100** (P0): Library crates use `thiserror` enums with structured context
  (spec, operation, OS error). The app surfaces errors in the Log panel and in
  per-file "error" states (never silent). Scripts log and respect
  `option stop-on-error`.
- **ARCH-101** (P0): Partial failures in batch operations continue with the remaining
  items and produce a summary ("Copied 10 of 12; 2 failed").

## 8. Testing strategy

- Unit tests per crate; property tests (`proptest`) for aligners (e.g. applying the
  diff to the left yields the right), hashing, file masks, script parsing.
- Golden tests: fixtures of file pairs with expected alignment/importance JSON.
- Integration tests for VFS back-ends against in-memory/local servers (FTP, SFTP,
  WebDAV, S3-compatible) started in CI.
- Benchmarks (`criterion`): 1M-line diff, 100k-file scan, 2 GB binary compare.
- GUI smoke tests via the command catalogue (every command has an enabled-state test).
