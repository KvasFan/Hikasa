# Architecture

## 0. Three layers: core, CLI, GUI

The product is organised in three strictly layered groups of crates:

```
┌──────────────────────────────────────────────────────────────────┐
│ gui/      hikasa-gui      desktop application (egui/eframe)      │  presents
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
├── Cargo.toml                 # workspace, version 0.1.90
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
│   └── hikasa-gui/            # desktop application (egui/eframe): windows, tabs, views, dialogs, IPC server
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
    ArchiveMember { archive: Box<Spec>, member: String }, // x.zip/inner/file; member "" is the root, x.zip/
    Snapshot { file: PathBuf, path: String },        // snapshot.hksnap/inner
    Remote { profile: ProfileRef, path: String },    // ftp://u@h/p, sftp://, ftps://, dav(s)://, s3://bucket/key, profile:Name?path
    Http(Url),                                       // single files only
    Clipboard,                                       // clipboard://
    Stdin,                                           // "-"
}
```

- **ARCH-020** (P0): Specs round-trip to a canonical string form used in sessions,
  on the command line and in scripts (`11-vfs-remote-archives.md` §2).
- **ARCH-021** (P0): Archive members nest (`a.zip/b.tar.gz/c.txt`). *M2 W5 (D-137):* `Spec::parse` never yields
  `ArchiveMember`: members appear through resolution (`hikasa_vfs::archive::resolve_spec`), `Vfs::child` and
  `Spec::archive_member`. `member` is `/`-separated on every platform; `member: ""` is the archive as a folder.

### 3.2 VFS (hikasa-vfs)

```
trait Vfs {
    fn stat(&self, spec:&Spec) -> Result<Entry>;
    fn list(&self, dir:&Spec, cancel:&Cancel) -> Result<Box<dyn Iterator<Item=Result<Entry>>>>;
    fn open_read(&self, spec:&Spec) -> Result<Box<dyn ReadSeek>>;   // streaming
    fn open_read_with(&self, spec:&Spec, cancel:&CancelToken) -> Result<Box<dyn ReadSeek>>;  // M2 W5: cancellable decode
    fn prepare(&self, spec:&Spec, ctl:&Ctl) -> Result<()>;          // M2 W5: the index of every archive around spec
    fn archive_format(&self, name:&str) -> Option<ArchiveFormat>;   // M2 W5: per the masks
    fn take_warnings(&self) -> Vec<Warning>;                        // M2 W5: raised since the last call
    fn begin(&self, at:&Spec) -> Result<()>;                        // M2 W5b: a batch on an archive, its whole chain
    fn finish(&self, at:&Spec, how:Finish) -> Result<()>;           // M2 W5b: Commit rebuilds once, Abort drops the overlay
    fn finish_with(&self, at:&Spec, how:Finish, ctl:&Ctl) -> Result<()>; // M2 W5b: finish with cancellation and progress
    fn create_archive(&self, spec:&Spec) -> Result<()>;             // M2 W5b: an empty archive, by its mask
    fn open_write(&self, spec:&Spec, opts:WriteOpts) -> Result<Box<dyn AtomicWrite>>;  // Write + Seek (M2 W5b) + commit
    fn capabilities(&self) -> Caps; // timestamps writable? attrs? unix mode? recycle bin? locks? crc provided?
    // M2 W3 (D-92): single-item mutating primitives, each behind a capability, default "unsupported";
    // they never follow the final link (D-94)
    fn create_dir(&self, spec:&Spec, parents:bool) -> Result<bool>;
    fn remove(&self, spec:&Spec) -> Result<()>;                 // file, link or empty folder
    fn rename(&self, from:&Spec, to:&Spec, replace:bool) -> Result<()>;
    fn set_times(&self, spec:&Spec, times:&SetTimes) -> Result<()>;
    fn set_attrs(&self, spec:&Spec, change:&AttrChange) -> Result<()>;
    fn create_symlink(&self, spec:&Spec, target:&OsStr, target_is_dir:bool) -> Result<()>;
    fn move_to_trash(&self, spec:&Spec) -> Result<()>;           // never a permanent delete
    fn copy_file_fast(&self, from:&Spec, to:&Spec, ..) -> Result<Option<u64>>;  // optional fast path
}
struct Entry { name, kind: File|Dir|Symlink, size, mtime, ctime?, attrs: DosAttrs|UnixMode, owner, group, link_target, crc32: Option<u32>, version: Option<VersionInfo> }
```

- **ARCH-030** (P0): Every back-end exposes capabilities; UI and operations degrade
  gracefully (e.g. "touch" greyed out when timestamps are read-only).
- **ARCH-031** (P0): Copy between back-ends is implemented generically (stream from
  source `open_read` to target `open_write`, then apply times/attrs if supported),
  with a fast path for local→local. *M2 W3:* implemented once in `hikasa_vfs::ops` over the
  primitives (plan, then execute with per-item results; copy, move, remove/trash, mkdir, touch).

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

**M2 implementation (binding interpretation; `design/m2-contracts.md` §1.2, D-80…D-85).** The generic
`Session<T>` above is one `hikasa_session::sessions::Session` with a type (`folder-compare`, `folder-sync`,
`text-compare`, `text-edit`, `table-compare`, `hex-compare`, `picture-compare`), its specs, its full rules
(typed `FolderRules`/`TextRules` for Folder and Text Compare and Text Edit, an opaque table for the other types
until their engine waves), the text rules of rules-based content tests (folder types), opaque `display` and
`view` tables, an optional parent link and, for folder types, per-pair and per-type overrides of children. The
file stores only what differs from the type's defaults. The effective rules (ARCH-040/041) are, in order: the
defaults of the type → the parent's `type` overrides for the child's type → the parent's `pair` override
matching the child's relative paths → the child's own changes (`effective_rules`). The scopes are
`apply_scope(View | Defaults | Pair | Type)`: `Defaults` writes `_defaults/<type>.toml` (rules only), `Pair` and
`Type` store an override in the parent (named or auto-saved), which a locked parent keeps in memory only.

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
  option applied to the work pool. *M1:* not applied (no approved OS API); see §5.1
  "ARCH-061 gap".

### 3.6 Commands (hikasa-gui, catalogue shared with the CLI)

- **ARCH-070** (P0): Every user action is a `Command { id, category, caption, hint,
  default_shortcut, secondary_shortcuts, menu: bool, toolbar: bool, enabled_when }`
  registered in a per-view-type catalogue (`reference/commands.md`). Menus, toolbars,
  context menus and the "Toolbars, Shortcuts, Menus" options page are all generated
  from this catalogue; shortcuts are user-configurable per view type.
  *M2 (D-65):* the catalogue data (id, menu, caption, default shortcut, description,
  help topic, status `implemented`/`planned`) lives in core,
  `core/hikasa-session/catalogue/commands.toml` (`hikasa_session::commands`), and
  `docs/reference/commands.md` is generated from it (`cargo xtask commands-doc`). The GUI
  keeps toolbar labels, radio groups and availability predicates next to its view
  models; its tests check that it executes exactly the `implemented` entries.
  *M2 W4a (D-109, D-119):* `hikasa_session::commands::effective(view, overrides)` merges the
  catalogue entries of a view's sections with `commands.toml` (shortcuts, menu and toolbar
  flags; an entry has an optional `toolbar` flag) into the view's effective table; the GUI
  reads captions, hints, menus and shortcuts from it and keeps only icons, short labels,
  groups, availability and context-menu areas. `Service::options()` and `registry()` return
  snapshots, and `Service::reload_settings()` swaps new ones in (later calls and jobs use them,
  running jobs keep theirs); the GUI's other settings calls are `settings_snapshot`,
  `settings_save`, `state_update`, `log_append` and `input_stamp` (typed forms of the
  `settings` commands or GUI-only state; no new `hikasa` subcommand).

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
  options, wait}`, `LoadSession{name}`, `LoadWorkspace{name}`, `ImportPackage{path}`, `RunScript{path,args}`,
  `ListViews`, `CloseView{id}`, `Ping`, `Shutdown`. Responses carry a view id, and
  when `wait` was requested a final `ViewClosed{id, result, exit_code}`. Unknown
  fields are ignored; the version is negotiated in `Hello`.
- **ARCH-092** (P0): `hikasa gui …` (CLI) implements the client side; `hikasa-gui`
  the server side. The CLI "wait" mode stays alive until the view closes and returns
  its exit code, which is what version-control systems need.

### 5.1 M1 implementation (binding interpretation; contract §1.8.6, §2.23, §4.4; D-46, D-47)

- **Transport.** `hikasa_session::ipc` runs on blocking `interprocess` local sockets: a
  Unix domain socket, or a named pipe on Windows. There is no async runtime in M1 (no
  tokio is approved): the server has one accept thread plus one thread per connection;
  handlers never block (the GUI posts to its event loop). Frames are a `u32`
  little-endian length (1 byte … 16 MiB) followed by UTF-8 JSON; EOF inside a frame is a
  lost connection. Timeouts are client-side: each request has one 10 s deadline for its
  immediate response, whatever else arrives (the client decodes frames on a reader thread
  and waits on a channel with a deadline), and there is none while waiting for
  `ViewClosed`; the launch retry budget is the sum of the launcher's sleeps (50 ms
  doubling to 500 ms, 10 s in total). No core code reads a clock or sleeps. The server
  writes every frame through a bounded per-connection queue (64 frames) drained by a
  writer thread with a 5 s send timeout, so a GUI never blocks on a client that stopped
  reading (that client is dropped). A frame with more than 262,144 JSON values is a
  protocol error, which bounds the decoder's memory per frame.
- **Forward compatibility (D-46).** `Request`, `Response` and `ViewKind` decode unknown
  `type`/kind values to an `Unknown` variant, and every frame is decoded in two phases:
  first as a JSON value, then its `id`, then the body. So a frame without a usable `id`
  is answered `Error{protocol}` with id 0 (and the connection closes); a known `type`
  whose fields do not decode is answered `Error{invalid_request}` with the request's own
  id; an unknown `type` is answered `Error{unsupported_request}` with its id; an unknown
  view kind `Error{unsupported_view}`. Unknown fields are ignored everywhere. Error codes
  and view results (`verdict`, `folder_verdict`, `reason`) travel as plain strings, so
  values added by a newer peer never break an older one; `ViewClosed::exit_code` is
  authoritative. A client receiving a response type it does not know treats it as a
  protocol error of the pending request. The first message is `Hello`; the server picks
  `min(its version, the client's)` and refuses (`incompatible_version`) when that is
  below the client's minimum. Every request gets exactly one immediate response; an
  `OpenView{wait: true}` gets one later `ViewClosed` with the same id, always written
  after the immediate `ViewOpened`. The server library enforces this: a view whose waiter
  the GUI drops without reporting is closed with reason `abandoned` and exit code 100, and
  stopping the server closes every view still awaited with reason `shutdown` (exit 100)
  and every connection, so `hikasa gui open --wait` never hangs on a running GUI. A
  protocol error closes the connection; an error the server sends with id 0 fails the
  client's pending request (exit 100).
- **Endpoint per user and configuration (D-47).** The endpoint is `hikasa.sock` in
  `$XDG_RUNTIME_DIR` (Linux; else Hikasa's own folder `<temp>/hikasa-<uid>`), in the
  per-user `$TMPDIR` (macOS), or the pipe `\\.\pipe\hikasa-<user>` (Windows). When the
  configuration folder is not the platform default (`--config-dir`, portable installs),
  the name gets the suffix `-<h8>`: the first 8 hex digits of the FNV-1a-64 hash of the
  absolute configuration folder (its bytes on Unix, UTF-16LE code units on Windows). So
  GUIs of different configurations neither collide nor share the single-instance lock,
  while every launch with the same configuration reaches the same instance. Unix socket
  paths longer than 100 bytes use a hashed name in `/tmp/hikasa-<uid>/`.
  `HIKASA_IPC_ENDPOINT` overrides the endpoint (tests, unusual setups). When the CLI
  launches the GUI it passes `--config-dir <dir>` for an explicit or portable
  configuration and the endpoint override in the child's environment, so the launched GUI
  listens where the CLI waits; `HIKASA_GUI_EXE` names the GUI executable to launch
  instead of the one next to the CLI or on `PATH` (tests).
- **Single instance and security.** The single-instance lock is an exclusive file lock
  on `<socket>.lock` (Windows: `%TEMP%\<pipe name>.lock`), held for the server's
  lifetime; it is race-free, unlike probing the socket, and lets a new server remove a
  stale socket safely (only a socket: any other file at the socket path is reported and
  never deleted). On Unix the **client verifies the socket folder before it connects**:
  the folder must belong to the current user and must not be writable by the group or by
  others (Hikasa's own fallback folder must also be a real folder, not a symbolic link,
  with mode 0700); otherwise the command fails with exit code 100 and never talks to,
  launches into or waits on that endpoint, so a socket another local user planted in a
  shared temporary folder is never contacted (no libc: `std` file metadata only). The
  server creates its own fallback folder with mode 0700 (create-new), refuses one it does
  not own, applies the same folder check before binding, gives the socket file mode 0600
  and checks that it belongs to the user; every connecting peer's effective uid must equal
  the socket owner's (connections of other users, and peers whose credentials cannot be
  read, are closed). `$XDG_RUNTIME_DIR` stays the preferred location. Windows relies on
  the per-user pipe name in M1; an owner-only DACL is a P1 item. `ServerHandle::shutdown`
  wakes the blocked accept by connecting to its own endpoint once, notifies the
  outstanding waiters, closes the connections and returns within about 2 s.
- **M1 requests.** The server itself answers `RunScript` with `unsupported_request` and
  `OpenView` of kinds other than `folder_compare` and `text_compare` with
  `unsupported_view`; everything else goes to the GUI's handler. (In M1 the server also
  answered `LoadSession` and `LoadWorkspace` itself; see the M2 additions below.) A second `hikasa-gui` launch that cannot take the lock forwards its
  command line as `OpenView` without waiting (`Service::forward_open`: specs made absolute
  against its own current directory, standard input spooled) and exits with 0.
- **M2 additions (D-89, additive to protocol v1).** `LoadSession{name, wait}` and
  `LoadWorkspace{name}` and (M2 W4c, D-127) `ImportPackage{path}` reach the GUI's handler. `ImportPackage` (an absolute
  path) answers `Ok` once the import wizard is shown in the window focused last; a GUI that answers
  `unsupported_request` is reported as for D-89. `LoadSession` answers `ViewOpened{view}`
  (and `ViewClosed` when waited; the server gives it a waiter like `OpenView`);
  `LoadWorkspace` answers `Views{views}` with the views it opened, after closing the open
  ones (SES-040). An unknown name is `Error{not_found}`, a session type the GUI cannot
  show `Error{unsupported_request}`. The CLI (`hikasa gui session`, `gui workspace`,
  `gui open <name>`) finds and checks the session or workspace locally first, and reports
  a GUI that answers `unsupported_request` (a 0.1 GUI) as "the running GUI does not
  support sessions; restart it" (exit 100). The GUI opens sessions through
  `Service::session_view` / `Service::workspace_views`: the session's specs and its
  effective rules as the `base_rules` of the view's request.
- **ARCH-061 gap.** Applying the comparison priority (Normal / Below normal / Idle) to
  the work pool needs an OS API (`setpriority`, `SetThreadPriority`), and no crate that
  offers one (`libc`, `thread-priority`) is approved. M1 therefore runs the service pool
  at normal priority. The service's pool builder (`hikasa_session::api::Service::new`) is
  the single place where worker threads start, and so the single place where a priority
  will be applied once a dependency is approved (Appendix C item 13 of the M1 contract).

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
