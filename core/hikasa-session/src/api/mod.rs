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
//! | `gui …`, `--gui` | [`Service::gui_open`], [`Service::gui_ping`], [`Service::gui_list`], [`Service::gui_close`], [`Service::gui_quit`] |

mod compare;
pub mod dto;
mod error;
pub mod gui;
pub mod human;
mod info;
mod report;
pub mod requests;
pub mod responses;
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
use hikasa_vfs::{Blob, LocalOptions, LocalVfs, StreamOptions, VfsError, VfsOp, VfsRouter};
use parking_lot::Mutex;

pub use error::ApiError;
pub use gui::{
    GuiLaunch, GuiLauncher, GuiOpenOutcome, GuiOpenReport, GuiOpenRequest, LaunchError, OpenArgs,
    ResolvedOpen, ViewOutcome, ViewRequest, ViewWords, find_gui_exe, gui_exe_candidates,
    infer_view_kind, normalize_legacy_args, open_usage, parse_open_args,
};
pub use report::ReportSettings;
pub use requests::{
    FolderCompareRequest, FormatTokensRequest, QuickMethod, QuickRequest, TextCompareRequest,
    spec_os_string,
};
pub use responses::{
    FolderCompareResponse, FormatChoice, PreparedFolderCompare, QuickResponse, TextCompareResponse,
};
pub use vocab::{
    parse_algorithm, parse_content_method, parse_folder_mode, parse_line_range,
    resolve_folder_display, resolve_text_display,
};

use crate::formats::FormatStore;
use crate::host::{Clock, EnvFacts};
use crate::ipc::Endpoint;
use crate::jobs::{
    JobControl, JobHandle, LossySink, PROGRESS_CAPACITY, PROGRESS_INTERVAL, run_guarded,
};
use crate::options::{ProgramOptions, load_options};
use crate::paths::{ConfigLocation, resolve_config_dir};

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
    options: ProgramOptions,
    ignored_keys: Vec<String>,
    registry: Arc<FormatRegistry>,
    /// The router without standard input.
    router: Arc<VfsRouter>,
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
    router: Arc<VfsRouter>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("config", &self.inner.config)
            .field("options", &self.inner.options)
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
        } = init;
        let exists = |p: &std::path::Path| p.is_file();
        let config = resolve_config_dir(&env, config_dir.as_deref(), &exists)?;
        let options_load = load_options(&config)?;
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
                options: options_load.options,
                ignored_keys: options_load.ignored_keys,
                registry: Arc::new(registry),
                router: Arc::new(router),
                pool: Arc::new(pool),
                clock,
                runner,
                jobs: JobIdAllocator::new(),
                startup_warnings,
                stdin_name,
                stdin_read: Mutex::new(stdin_read),
                stdin: OnceLock::new(),
            }),
        })
    }

    /// Configuration folder.
    pub fn config(&self) -> &ConfigLocation {
        &self.inner.config
    }

    /// Program options.
    pub fn options(&self) -> &ProgramOptions {
        &self.inner.options
    }

    /// Format registry.
    pub fn registry(&self) -> &Arc<FormatRegistry> {
        &self.inner.registry
    }

    /// Warnings collected during construction (unknown config sections, skipped user formats).
    pub fn startup_warnings(&self) -> &[Warning] {
        &self.inner.startup_warnings
    }

    /// Option keys ignored while loading `options.toml` (logged with `--verbose`, D-31).
    pub fn ignored_option_keys(&self) -> &[String] {
        &self.inner.ignored_keys
    }

    /// The captured host facts (additive).
    pub fn env(&self) -> &EnvFacts {
        &self.inner.env
    }

    /// The VFS router (local files, and standard input once a `-` spec was parsed; additive). GUIs read
    /// through it.
    pub fn vfs(&self) -> &VfsRouter {
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
        self.inner.options.stream_options(threads)
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
    /// error reading it, or no source, is an error here.
    pub fn parse_spec(&self, input: &OsStr) -> Result<Spec, ApiError> {
        let spec = Spec::parse_os(input, &self.spec_context())?;
        if spec == Spec::Stdin {
            self.stdin_state()?;
        }
        Ok(spec)
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
            let now = self.inner.clock.now().timestamp();
            let nanos = u32::try_from(now.subsec_nanosecond())
                .unwrap_or(0)
                .min(999_999_999);
            let mtime = FileTime::from_unix(now.as_second(), nanos).ok_or_else(|| {
                StdinUnavailable::Failed("the clock's time is out of range".to_owned())
            })?;
            let blob = Blob {
                bytes: Arc::from(bytes),
                mtime,
                name: self.inner.stdin_name.clone(),
            };
            let router =
                VfsRouter::new(LocalVfs::new(LocalOptions::default())).with_stdin(blob.clone());
            Ok(StdinState {
                blob,
                router: Arc::new(router),
            })
        });
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

    /// The router as a shared value: with standard input once it was read.
    pub(crate) fn router(&self) -> &Arc<VfsRouter> {
        match self.inner.stdin.get() {
            Some(Ok(state)) => &state.router,
            Some(Err(_)) | None => &self.inner.router,
        }
    }
}
