//! The service API (`hikasa_session::api`, ARCH-013; §1.8.4): one [`Service`] per process, used identically
//! by the CLI and the GUI.
//!
//! The service owns everything a comparison needs besides its arguments: the captured host facts, the
//! configuration folder, the program options, the format registry, the VFS router (with the stdin blob), the
//! explicit rayon pool (`hikasa-worker-N`; the engines never touch the global pool, T7), the injected clock and
//! the external-conversion runner. Each method body runs behind the panic boundary ([`run_guarded`], §5.6), and
//! every result carries its exit code (§2.21).
//!
//! | Command | Method |
//! |---|---|
//! | `quick` | [`Service::quick`] |
//! | `text compare` | [`Service::text_compare`], [`Service::render_text`], [`Service::write_output`] |
//! | `folder compare` | [`Service::folder_prepare`] + [`Service::folder_run`] = [`Service::folder_compare`], [`Service::render_folder`] |
//! | `format list/show/detect/tokens` | [`Service::format_list`], [`Service::format_show`], [`Service::format_detect`], [`Service::format_tokens`] |
//! | `vfs ls/stat/cat` | [`Service::vfs_ls`], [`Service::vfs_stat`], [`Service::vfs_cat`] |
//! | `vfs cp/mv/rm/mkdir/touch` (M2 W3) | [`Service::vfs_plan`] (`--dry-run`), [`Service::vfs_run`] |
//! | `clipboard://` (M2 W3) | [`Service::parse_spec`] (captured once), [`Service::read_clipboard`] |
//! | `settings get/set/unset/list/restore-defaults` | [`Service::settings_get`], [`Service::settings_set`], [`Service::settings_unset`], [`Service::settings_list`], [`Service::settings_restore_defaults`] |
//! | `format new/edit/delete/import/export/enable/disable/move/restore` | [`Service::format_new`], [`Service::format_edit`], [`Service::format_delete`], [`Service::format_import`], [`Service::format_export`], [`Service::format_enable`], [`Service::format_move`], [`Service::format_restore`] |
//! | `session list/show/new/edit/delete/rename/lock/unlock/mkdir/rmdir/run` | [`Service::session_list`], [`Service::session_show`], [`Service::session_new`], [`Service::session_edit`], [`Service::session_delete`], [`Service::session_rename`], [`Service::session_lock`], [`Service::session_mkdir`], [`Service::session_rmdir`], [`Service::session_run_request`] |
//! | `workspace list/show/delete/rename` | [`Service::workspace_list`], [`Service::workspace_show`], [`Service::workspace_delete`], [`Service::workspace_rename`] |
//! | `settings export/import`, `restore-defaults --category` | [`Service::settings_export`], [`Service::settings_import`], [`Service::settings_restore_categories`] |
//! | GUI settings (M2 W4a) | [`Service::settings_snapshot`], [`Service::settings_save`], [`Service::reload_settings`], [`Service::state_update`], [`Service::log_append`], [`Service::input_stamp`] |
//! | `gui …`, `--gui` | [`Service::gui_open`], [`Service::gui_ping`], [`Service::gui_list`], [`Service::gui_close`], [`Service::gui_quit`] |

mod compare;
pub mod dto;
mod error;
pub mod gui;
pub mod human;
mod info;
pub mod package;
mod report;
pub mod requests;
pub mod responses;
pub mod sessions;
pub mod settings;
pub mod vfs_ops;
pub mod vocab;

use std::ffi::OsStr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use hikasa_core::{
    CancelToken, FileTime, JobIdAllocator, JobKind, ProgressSink, Spec, SpecContext, ThrottledSink,
    Warning,
};
use hikasa_formats::{FormatRegistry, ProcessRunner, RegexLimits, TextEncoding};
use hikasa_vfs::{
    Blob, BlobFailure, LocalOptions, LocalVfs, StreamOptions, VfsError, VfsOp, VfsRouter,
};
use parking_lot::{Mutex, RwLock};

pub use error::ApiError;
pub use gui::{
    GuiLaunch, GuiLauncher, GuiOpenOutcome, GuiOpenReport, GuiOpenRequest, LaunchError, OpenArgs,
    ResolvedOpen, ViewOutcome, ViewRequest, ViewWords, find_gui_exe, gui_exe_candidates,
    infer_view_kind, normalize_legacy_args, open_usage, parse_open_args,
};
pub use package::{PlanDto, PlanItemDto, SettingsImportDto};
pub use report::ReportSettings;
pub use requests::{
    FolderCompareRequest, FormatTokensRequest, QuickMethod, QuickRequest, TextCompareRequest,
    spec_os_string,
};
pub use responses::{
    FolderCompareResponse, FormatChoice, PreparedFolderCompare, QuickResponse, TextCompareResponse,
};
pub use sessions::{
    AutoSavedDto, ScopeWritesDto, SessionEdit, SessionItemDto, SessionListDto, SessionRunRequest,
    SessionShowDto, SessionTarget, SessionView, SessionWriteDto, SingleArg, WorkspaceItemDto,
    WorkspaceListDto, WorkspaceLoadDto, WorkspaceShowDto, WorkspaceWriteDto, parse_assignment,
};
pub use settings::{
    FormatExportDto, FormatRestoreDto, FormatWriteDto, MoveTo, RestoreFileDto, SettingsEntryDto,
    SettingsListDto, SettingsRestoreDto, SettingsSetDto, SettingsValueDto, parse_format_kind,
};
pub use vfs_ops::{
    TouchTime, VfsOpDto, VfsOpItemDto, VfsOpRequest, ends_with_separator, parse_link_policy,
    parse_overwrite, parse_time,
};
pub use vocab::{
    parse_algorithm, parse_content_method, parse_folder_mode, parse_line_range,
    resolve_folder_display, resolve_text_display,
};

use crate::clipboard::{
    ClipboardContent, ClipboardError, ClipboardProvider, NoClipboard, clipboard_blob,
};
use crate::formats::FormatStore;
use crate::host::{Clock, EnvFacts};
use crate::ipc::Endpoint;
use crate::jobs::{
    JobControl, JobHandle, LossySink, PROGRESS_CAPACITY, PROGRESS_INTERVAL, run_guarded,
};
use crate::options::{ProgramOptions, load_options_with};
use crate::paths::{ConfigLocation, resolve_config_dir};
use crate::settings::LoadMode;

/// Version of this build (`CARGO_PKG_VERSION`), as written in `hikasa_version` and the client/generator names.
pub const HIKASA_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Largest number of threads one mapped CRC or binary comparison uses (`min(4, pool size)`, §5.1).
pub const MAX_STREAM_THREADS: usize = 4;

/// The function that reads all of standard input.
type StdinRead = Box<dyn FnOnce() -> std::io::Result<Vec<u8>> + Send>;

/// Standard input as the adapter provides it (D-15, D-49; wave-4 review §1.8.7): its `--stdin-name` hint and
/// a function that reads it. The service calls the function at most once, the first time a `-` spec is
/// parsed ([`Service::parse_spec`]), and stamps the content with the injected clock; commands without `-` and
/// `format detect -` (which needs the name only) never read standard input. So adapters pass a source
/// unconditionally and never decide themselves which arguments are specs.
pub struct StdinSource {
    /// The `--stdin-name` hint (format detection, the spool file name).
    pub name: Option<String>,
    read: StdinRead,
}

impl StdinSource {
    /// A source that calls `read` on first use (the CLI: `std::io::stdin().read_to_end`).
    pub fn new(
        name: Option<String>,
        read: impl FnOnce() -> std::io::Result<Vec<u8>> + Send + 'static,
    ) -> StdinSource {
        StdinSource {
            name,
            read: Box::new(read),
        }
    }

    /// A source with known content (tests, embedders).
    pub fn from_bytes(bytes: impl Into<Vec<u8>>, name: Option<String>) -> StdinSource {
        let bytes = bytes.into();
        StdinSource::new(name, move || Ok(bytes))
    }
}

impl std::fmt::Debug for StdinSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdinSource")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Service construction parameters (captured by the adapter).
pub struct ServiceInit {
    /// Ambient facts.
    pub env: EnvFacts,
    /// `--config-dir`.
    pub config_dir: Option<PathBuf>,
    /// Worker threads (None = available parallelism, min 1).
    pub threads: Option<NonZeroUsize>,
    /// Standard input with the `--stdin-name` hint, read lazily on the first `-` spec ([`StdinSource`]).
    /// Changed in the wave-4 review (was an eagerly read `Blob`).
    pub stdin: Option<StdinSource>,
    /// Clock.
    pub clock: Arc<dyn Clock>,
    /// External conversion runner (`StdProcessRunner` in adapters, fakes in tests).
    pub runner: Arc<dyn ProcessRunner>,
    /// How `options.toml` is read: `Strict` for every command, `Lenient` for the `settings` commands (D-71;
    /// added in M2 W1).
    pub options_mode: LoadMode,
    /// The clipboard behind `clipboard://` (M2 W3, D-100): read only when a `clipboard://` spec is parsed (once
    /// per service) and by [`Service::read_clipboard`]. Adapters pass `host::clipboard_provider`; tests a
    /// `FixedClipboard`. [`ServiceInit::new`] sets [`NoClipboard`].
    pub clipboard: Arc<dyn ClipboardProvider>,
}

impl ServiceInit {
    /// Parameters with the default configuration folder, the default thread count and no stdin (additive).
    pub fn new(
        env: EnvFacts,
        clock: Arc<dyn Clock>,
        runner: Arc<dyn ProcessRunner>,
    ) -> ServiceInit {
        ServiceInit {
            env,
            config_dir: None,
            threads: None,
            stdin: None,
            clock,
            runner,
            options_mode: LoadMode::Strict,
            clipboard: Arc::new(NoClipboard),
        }
    }
}

impl std::fmt::Debug for ServiceInit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceInit")
            .field("env", &self.env)
            .field("config_dir", &self.config_dir)
            .field("threads", &self.threads)
            .field("stdin", &self.stdin)
            .field("options_mode", &self.options_mode)
            .finish_non_exhaustive()
    }
}

/// Operation context for exit-code mapping (§2.21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiOp {
    /// `quick`.
    Quick,
    /// `text compare`.
    TextCompare,
    /// `folder compare`.
    FolderCompare,
    /// `format …`.
    Format,
    /// `vfs …`.
    Vfs,
    /// `gui open` and `--gui`.
    GuiOpen {
        /// `--wait`.
        wait: bool,
    },
    /// `gui ping|list|close|quit`.
    GuiControl,
    /// Everything else (`completions`, `version`, configuration failures before a command runs).
    Other,
}

/// The service. `Send + Sync`; cheap to clone (`Arc` inside); one per process, shared by all CLI commands,
/// GUI views and jobs.
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}

struct Inner {
    env: EnvFacts,
    config: ConfigLocation,
    /// The options of `options.toml`, their ignored keys and the format registry, behind one lock: replaced
    /// together by [`Service::reload_settings`] (D-119), so a reader never sees half a reload.
    loaded: RwLock<Arc<LoadedSettings>>,
    /// Held across the whole of a reload (load and swap): reloads never interleave or finish out of order.
    reload: Mutex<()>,
    /// The router: local files, plus standard input and the clipboard once captured (replaced on capture).
    router: RwLock<Arc<VfsRouter>>,
    pool: Arc<rayon::ThreadPool>,
    clock: Arc<dyn Clock>,
    runner: Arc<dyn ProcessRunner>,
    jobs: JobIdAllocator,
    startup_warnings: Vec<Warning>,
    stdin_name: Option<String>,
    /// The unread source (taken by the first `-`).
    stdin_read: Mutex<Option<StdinRead>>,
    /// Standard input once read: the blob and the router that serves it (or why it is unavailable).
    stdin: OnceLock<Result<StdinState, StdinUnavailable>>,
    /// The clipboard provider (M2 W3).
    clipboard: Arc<dyn ClipboardProvider>,
    /// The clipboard snapshot: `None` until the first `clipboard://` spec (or `read_clipboard`).
    clipboard_state: Mutex<Option<Result<Blob, BlobFailure>>>,
}

/// The settings a [`Service`] holds in memory (D-119): one consistent view, taken with
/// [`Service::loaded_settings`] and replaced as a whole by [`Service::reload_settings`].
#[derive(Debug)]
pub struct LoadedSettings {
    /// `options.toml`.
    pub options: Arc<ProgramOptions>,
    /// The keys of `options.toml` that were ignored while loading it.
    pub ignored_keys: Vec<String>,
    /// The format registry.
    pub registry: Arc<FormatRegistry>,
}

/// Why standard input is unavailable.
#[derive(Debug, Clone)]
enum StdinUnavailable {
    /// The adapter passed no source.
    Missing,
    /// Reading failed.
    Failed(String),
}

/// Standard input after it was read.
struct StdinState {
    blob: Blob,
}

/// What `Service::read_clipboard` found (TC-065 data).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClipboardDto {
    /// `"text"` or `"image"`.
    pub kind: &'static str,
    /// Bytes of the blob (UTF-8 text, or the PNG).
    pub size: u64,
    /// The name hint (`clipboard.png` for images).
    pub name: Option<String>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("config", &self.inner.config)
            .field("options", &*self.inner.loaded.read().options)
            .field("threads", &self.inner.pool.current_num_threads())
            .finish_non_exhaustive()
    }
}

impl Service {
    /// Resolves config, loads options and formats, builds the VFS router and the thread pool
    /// (threads named `hikasa-worker-N`).
    ///
    /// Errors: the configuration folder cannot be determined or is relative without a current directory;
    /// `options.toml` or `formats/order.toml` is broken or from a newer version; the pool cannot be built.
    /// Broken user format files are skipped with warnings ([`Service::startup_warnings`]).
    pub fn new(init: ServiceInit) -> Result<Service, ApiError> {
        run_guarded(|| Service::build(init))
    }

    fn build(init: ServiceInit) -> Result<Service, ApiError> {
        let ServiceInit {
            env,
            config_dir,
            threads,
            stdin,
            clock,
            runner,
            options_mode,
            clipboard,
        } = init;
        let exists = |p: &std::path::Path| p.is_file();
        let config = resolve_config_dir(&env, config_dir.as_deref(), &exists)?;
        let options_load = load_options_with(&config, options_mode)?;
        let (registry, format_warnings) = FormatStore::new(&config).load(RegexLimits::default())?;
        let mut startup_warnings = options_load.warnings;
        startup_warnings.extend(format_warnings);
        for key in &options_load.ignored_keys {
            tracing::debug!(target: "hikasa::options", "options.toml: ignored key `{key}`");
        }
        // One channel for startup warnings (wave-4 review): they are data in `startup_warnings()`, which the
        // adapters put into every JSON envelope and print once; the log only records them at debug level.
        for warning in &startup_warnings {
            tracing::debug!(target: "hikasa::config", "{}: {}", warning.code, warning.message);
        }
        let threads = threads
            .map(NonZeroUsize::get)
            .or_else(|| {
                std::thread::available_parallelism()
                    .ok()
                    .map(NonZeroUsize::get)
            })
            .unwrap_or(1)
            .max(1);
        // ARCH-061 (comparison priority) would be applied here, the single place where worker threads start;
        // M1 has no approved OS API for it (gap, Appendix C item 13).
        // A panic in work spawned on the pool (`pool().spawn`) would abort the process without a handler
        // (rayon's default): the handler logs it instead, the panicking task ends, and the worker and every other
        // task keep running (§5.6 amendment). Panics inside `install`/`join`/`scope` still propagate to the
        // caller, where `run_guarded` turns them into `ApiError::Internal`.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("hikasa-worker-{i}"))
            .panic_handler(|payload| {
                tracing::error!(
                    target: "hikasa",
                    "a task on the worker pool panicked and was dropped: {}",
                    crate::jobs::panic_message(payload.as_ref())
                );
            })
            .build()
            .map_err(|e| ApiError::Internal(format!("cannot start the worker threads: {e}")))?;
        let router = VfsRouter::new(LocalVfs::new(LocalOptions::default()));
        let (stdin_name, stdin_read) = match stdin {
            Some(source) => (source.name, Some(source.read)),
            None => (None, None),
        };
        tracing::debug!(
            target: "hikasa::service",
            "configuration {} ({:?}), {threads} worker threads",
            config.dir.display(),
            config.mode
        );
        Ok(Service {
            inner: Arc::new(Inner {
                env,
                config,
                loaded: RwLock::new(Arc::new(LoadedSettings {
                    options: Arc::new(options_load.options),
                    ignored_keys: options_load.ignored_keys,
                    registry: Arc::new(registry),
                })),
                reload: Mutex::new(()),
                router: RwLock::new(Arc::new(router)),
                pool: Arc::new(pool),
                clock,
                runner,
                jobs: JobIdAllocator::new(),
                startup_warnings,
                stdin_name,
                stdin_read: Mutex::new(stdin_read),
                stdin: OnceLock::new(),
                clipboard,
                clipboard_state: Mutex::new(None),
            }),
        })
    }

    /// Configuration folder.
    pub fn config(&self) -> &ConfigLocation {
        &self.inner.config
    }

    /// Program options: a snapshot (M2 W4a, D-119: was `&ProgramOptions`). Calls and jobs started after a
    /// [`Service::reload_settings`] see the new values; a running job keeps the snapshot it took.
    pub fn options(&self) -> Arc<ProgramOptions> {
        Arc::clone(&self.inner.loaded.read().options)
    }

    /// Format registry: a snapshot (M2 W4a, D-119: was `&Arc<FormatRegistry>`), like [`Service::options`].
    pub fn registry(&self) -> Arc<FormatRegistry> {
        Arc::clone(&self.inner.loaded.read().registry)
    }

    /// The options, their ignored keys and the registry as one consistent snapshot (D-119): never half of a
    /// [`Service::reload_settings`].
    pub fn loaded_settings(&self) -> Arc<LoadedSettings> {
        Arc::clone(&self.inner.loaded.read())
    }

    /// Re-reads `options.toml` (leniently: a bad value is its default, with warning `invalid_config_value`) and
    /// the format registry, and swaps them atomically (M2 W4a, D-119): later calls and jobs use the new values,
    /// running jobs keep theirs. Options, their ignored keys and the registry are replaced together under one
    /// lock, and reloads are serialized (a reload holds a mutex from its first read to its swap), so concurrent
    /// reloads never finish out of order. A file that cannot be read at all (a newer `schema_version`, a broken
    /// `order.toml`) keeps the previous value and is reported as a warning. Returns the warnings of both loads.
    pub fn reload_settings(&self) -> Vec<Warning> {
        let _serial = self.inner.reload.lock();
        let current = self.loaded_settings();
        let mut warnings = Vec::new();
        let kept = |what: &str, e: &dyn std::fmt::Display| Warning {
            code: hikasa_core::WarningCode::InvalidConfigValue,
            message: format!("{what}: {e} (the previous settings stay in use)"),
            spec: None,
        };
        let (options, ignored_keys) = match load_options_with(&self.inner.config, LoadMode::Lenient)
        {
            Ok(load) => {
                warnings.extend(load.warnings);
                (Arc::new(load.options), load.ignored_keys)
            }
            Err(e) => {
                warnings.push(kept(crate::options::OPTIONS_FILE, &e));
                (Arc::clone(&current.options), current.ignored_keys.clone())
            }
        };
        let registry = match FormatStore::new(&self.inner.config).load(RegexLimits::default()) {
            Ok((registry, format_warnings)) => {
                warnings.extend(format_warnings);
                Arc::new(registry)
            }
            Err(e) => {
                warnings.push(kept("formats", &e));
                Arc::clone(&current.registry)
            }
        };
        *self.inner.loaded.write() = Arc::new(LoadedSettings {
            options,
            ignored_keys,
            registry,
        });
        warnings
    }

    /// Warnings collected during construction (unknown config sections, skipped user formats).
    pub fn startup_warnings(&self) -> &[Warning] {
        &self.inner.startup_warnings
    }

    /// Option keys ignored while loading `options.toml` (logged with `--verbose`, D-31); refreshed by
    /// [`Service::reload_settings`] (M2 W4a: was `&[String]`).
    pub fn ignored_option_keys(&self) -> Vec<String> {
        self.inner.loaded.read().ignored_keys.clone()
    }

    /// The captured host facts (additive).
    pub fn env(&self) -> &EnvFacts {
        &self.inner.env
    }

    /// The VFS router (local files, and standard input and the clipboard once captured; additive). GUIs read
    /// through it. A snapshot (M2 W3: was `&VfsRouter`): the router changes when a blob is captured.
    pub fn vfs(&self) -> Arc<VfsRouter> {
        self.router()
    }

    /// The service's worker pool (additive): GUI work that needs parallelism runs on it, never on the rayon
    /// global pool (T7). A panic in a task given to `pool().spawn` is logged and drops that task only (the pool
    /// has a panic handler, §5.6 amendment); work whose panic must become an error goes through
    /// [`Service::spawn`] or `pool().install`/`scope` inside a service call.
    pub fn pool(&self) -> &rayon::ThreadPool {
        &self.inner.pool
    }

    /// The injected clock (additive).
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.inner.clock
    }

    /// Stream options of the binary and CRC comparisons: the configured buffer size and
    /// `min(4, pool size)` mapped-chunk threads (§5.1; additive).
    pub fn stream_options(&self) -> StreamOptions {
        let threads = self
            .inner
            .pool
            .current_num_threads()
            .clamp(1, MAX_STREAM_THREADS);
        self.options().stream_options(threads)
    }

    /// `"hikasa <version>"`: the report generator and IPC client name (additive).
    pub fn generator(&self) -> String {
        format!("hikasa {HIKASA_VERSION}")
    }

    /// Spec context from the captured facts.
    pub fn spec_context(&self) -> SpecContext {
        SpecContext {
            style: self.inner.env.platform.path_style(),
            cwd: self.inner.env.cwd.clone(),
            home: self.inner.env.home.clone(),
        }
    }

    /// Parses a user spec. A `-` spec reads standard input now, once per service ([`StdinSource`]): an
    /// error reading it, or no source, is an error here. A `clipboard://` spec reads the clipboard now, once per
    /// service (M2 W3, D-100); a failed read is not an error here: the router answers it when the spec is read
    /// (`unsupported`, `not_found` or `io`).
    pub fn parse_spec(&self, input: &OsStr) -> Result<Spec, ApiError> {
        let spec = Spec::parse_os(input, &self.spec_context())?;
        match spec {
            Spec::Stdin => {
                self.stdin_state()?;
            }
            Spec::Clipboard => {
                let mut state = self.inner.clipboard_state.lock();
                if state.is_none() {
                    *state = Some(self.capture_clipboard());
                    drop(state);
                    self.rebuild_router();
                }
            }
            _ => {}
        }
        Ok(spec)
    }

    /// Reads the clipboard **again** and replaces the snapshot later `clipboard://` specs see (TC-065 data; the
    /// GUI before it opens a `clipboard://` view, D-101). Errors: no clipboard → `unsupported`, empty →
    /// `not_found`, a failed read → `io`.
    pub fn read_clipboard(&self) -> Result<ClipboardDto, ApiError> {
        run_guarded(|| {
            let captured = self.capture_clipboard();
            *self.inner.clipboard_state.lock() = Some(captured.clone());
            self.rebuild_router();
            match captured {
                Ok(blob) => Ok(ClipboardDto {
                    kind: if blob.name.is_some() { "image" } else { "text" },
                    size: blob.bytes.len() as u64,
                    name: blob.name,
                }),
                Err(failure) => Err(ApiError::Vfs(failure.error(&Spec::Clipboard))),
            }
        })
    }

    /// One read of the provider, stamped with the injected clock (T1).
    fn capture_clipboard(&self) -> Result<Blob, BlobFailure> {
        let content = self.inner.clipboard.read().map_err(|e| match e {
            ClipboardError::Unavailable(_) => BlobFailure::Unavailable(e.to_string()),
            ClipboardError::Empty => BlobFailure::Empty,
            ClipboardError::Failed(_) => BlobFailure::Failed(e.to_string()),
            ClipboardError::TooLarge(limit) => BlobFailure::TooLarge(limit),
        })?;
        let mtime = self
            .clock_file_time()
            .ok_or_else(|| BlobFailure::Failed("the clock's time is out of range".to_owned()))?;
        if let ClipboardContent::Text(text) = &content {
            tracing::debug!(target: "hikasa::clipboard", "clipboard text, {} bytes", text.len());
        }
        clipboard_blob(content, mtime).map_err(|e| match e {
            VfsError::NotFound { .. } => BlobFailure::Empty,
            VfsError::TooLarge { limit, .. } => BlobFailure::TooLarge(limit),
            other => BlobFailure::Failed(other.to_string()),
        })
    }

    /// The injected clock's now as a `FileTime`.
    pub(crate) fn clock_file_time(&self) -> Option<FileTime> {
        let now = self.inner.clock.now().timestamp();
        let nanos = u32::try_from(now.subsec_nanosecond())
            .unwrap_or(0)
            .min(999_999_999);
        FileTime::from_unix(now.as_second(), nanos)
    }

    /// Rebuilds the router from the captured blobs.
    fn rebuild_router(&self) {
        let mut router = VfsRouter::new(LocalVfs::new(LocalOptions::default()));
        if let Some(blob) = self.stdin() {
            router = router.with_stdin(blob.clone());
        }
        match &*self.inner.clipboard_state.lock() {
            Some(Ok(blob)) => router = router.with_clipboard(blob.clone()),
            Some(Err(failure)) => router = router.with_clipboard_failure(failure.clone()),
            None => {}
        }
        *self.inner.router.write() = Arc::new(router);
    }

    /// The `--stdin-name` hint (additive): format detection of `-` uses it without reading standard input.
    pub fn stdin_name(&self) -> Option<&str> {
        self.inner.stdin_name.as_deref()
    }

    /// Standard input, read on first use.
    fn stdin_state(&self) -> Result<&StdinState, ApiError> {
        let state = self.inner.stdin.get_or_init(|| {
            let read = self
                .inner
                .stdin_read
                .lock()
                .take()
                .ok_or(StdinUnavailable::Missing)?;
            let bytes = read().map_err(|e| {
                StdinUnavailable::Failed(format!("cannot read standard input: {e}"))
            })?;
            let mtime = self.clock_file_time().ok_or_else(|| {
                StdinUnavailable::Failed("the clock's time is out of range".to_owned())
            })?;
            let blob = Blob {
                bytes: Arc::from(bytes),
                mtime,
                name: self.inner.stdin_name.clone(),
            };
            Ok(StdinState { blob })
        });
        if state.is_ok() {
            self.rebuild_router();
        }
        state.as_ref().map_err(|unavailable| match unavailable {
            StdinUnavailable::Missing => {
                ApiError::InvalidArgument("`-` needs standard input, which was not provided".into())
            }
            StdinUnavailable::Failed(message) => ApiError::Vfs(VfsError::from_io(
                "-",
                VfsOp::Read,
                std::io::Error::other(message.clone()),
            )),
        })
    }

    /// The IPC endpoint of this user and configuration (§2.23).
    pub fn endpoint(&self) -> Result<Endpoint, ApiError> {
        Endpoint::for_config(&self.inner.env, &self.inner.config).map_err(ApiError::from)
    }

    /// Creates a job control for a direct (blocking) call.
    pub fn job(&self, kind: JobKind, progress: Arc<dyn ProgressSink>) -> JobControl {
        JobControl {
            id: self.inner.jobs.next(),
            kind,
            cancel: CancelToken::new(),
            progress: Arc::new(ThrottledSink::new(progress, PROGRESS_INTERVAL)),
        }
    }

    /// Runs `f` on a new thread named `hikasa-job-<id>` with a fresh `CancelToken`, a lossy progress channel
    /// and the panic boundary (`run_guarded`). The GUI's way to start every long operation.
    ///
    /// The body runs on the job thread itself and uses the pool only through the service calls it makes (their
    /// engines take the pool explicitly), so a long job never pins a worker for its whole duration and a small
    /// view started next to long ones gets its first screen at once (§5.1 amendment, wave-4 review). If the
    /// thread cannot be started, the handle's result is an `ApiError::Internal` at once.
    pub fn spawn<T: Send + 'static>(
        &self,
        kind: JobKind,
        title: String,
        f: impl FnOnce(&Service, &JobControl) -> Result<T, ApiError> + Send + 'static,
    ) -> JobHandle<T> {
        let id = self.inner.jobs.next();
        let cancel = CancelToken::new();
        let (progress_tx, progress_rx) = crossbeam_channel::bounded(PROGRESS_CAPACITY);
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let job = JobControl {
            id,
            kind,
            cancel: cancel.clone(),
            progress: Arc::new(ThrottledSink::new(
                LossySink(progress_tx),
                PROGRESS_INTERVAL,
            )),
        };
        let service = self.clone();
        let fallback = result_tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("hikasa-job-{id}"))
            .spawn(move || {
                let result = run_guarded(|| f(&service, &job));
                let _ = result_tx.send(result);
            });
        if let Err(e) = spawned {
            let _ = fallback.send(Err(ApiError::Internal(format!(
                "cannot start the job thread: {e}"
            ))));
        }
        drop(fallback);
        JobHandle::new(id, kind, title, cancel, progress_rx, result_rx)
    }

    /// The "current locale" encoding (D-42): the captured codeset when it names a known encoding, else UTF-8.
    pub(crate) fn locale_encoding(&self) -> TextEncoding {
        self.inner
            .env
            .locale_encoding
            .as_deref()
            .and_then(TextEncoding::for_label)
            .unwrap_or(TextEncoding::UTF_8)
    }

    /// The external-conversion runner and its temporary folder (`EnvFacts::temp_dir`).
    pub(crate) fn external(&self) -> (&std::path::Path, &dyn ProcessRunner) {
        (self.inner.env.temp_dir.as_path(), &*self.inner.runner)
    }

    /// The runner as a shared value (for the `RulesComparator`).
    pub(crate) fn runner(&self) -> &Arc<dyn ProcessRunner> {
        &self.inner.runner
    }

    /// The stdin blob, once a `-` spec was parsed.
    pub(crate) fn stdin(&self) -> Option<&Blob> {
        match self.inner.stdin.get() {
            Some(Ok(state)) => Some(&state.blob),
            Some(Err(_)) | None => None,
        }
    }

    /// The router as a shared value: with standard input and the clipboard once captured.
    pub(crate) fn router(&self) -> Arc<VfsRouter> {
        Arc::clone(&self.inner.router.read())
    }
}
