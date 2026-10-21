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
mod info;
mod report;
pub mod requests;
pub mod responses;
pub mod vocab;

use std::ffi::OsStr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{
    CancelToken, JobIdAllocator, JobKind, ProgressSink, Spec, SpecContext, ThrottledSink, Warning,
};
use hikasa_formats::{FormatRegistry, ProcessRunner, RegexLimits, TextEncoding};
use hikasa_vfs::{Blob, LocalOptions, LocalVfs, StreamOptions, VfsRouter};

pub use error::ApiError;
pub use gui::{
    GuiLauncher, GuiOpenOutcome, GuiOpenReport, GuiOpenRequest, LaunchError, OpenArgs,
    gui_exe_candidates, infer_view_kind, normalize_legacy_args, parse_open_args,
};
pub use report::ReportSettings;
pub use requests::{
    FolderCompareRequest, FormatTokensRequest, QuickMethod, QuickRequest, TextCompareRequest,
};
pub use responses::{
    FolderCompareResponse, FormatChoice, PreparedFolderCompare, QuickResponse, TextCompareResponse,
};
pub use vocab::parse_folder_mode;

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

/// Service construction parameters (captured by the adapter).
pub struct ServiceInit {
    /// Ambient facts.
    pub env: EnvFacts,
    /// `--config-dir`.
    pub config_dir: Option<PathBuf>,
    /// Worker threads (None = available parallelism, min 1).
    pub threads: Option<NonZeroUsize>,
    /// Stdin content (with the `--stdin-name` hint), read by the adapter only when a spec is "-".
    pub stdin: Option<Blob>,
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
    router: Arc<VfsRouter>,
    pool: Arc<rayon::ThreadPool>,
    clock: Arc<dyn Clock>,
    runner: Arc<dyn ProcessRunner>,
    jobs: JobIdAllocator,
    startup_warnings: Vec<Warning>,
    stdin: Option<Blob>,
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
        for warning in &startup_warnings {
            tracing::warn!(target: "hikasa::config", "{}: {}", warning.code, warning.message);
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
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("hikasa-worker-{i}"))
            .build()
            .map_err(|e| ApiError::Internal(format!("cannot start the worker threads: {e}")))?;
        let mut router = VfsRouter::new(LocalVfs::new(LocalOptions::default()));
        if let Some(blob) = stdin.clone() {
            router = router.with_stdin(blob);
        }
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
                stdin,
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

    /// The VFS router (local files, the stdin blob; additive). GUIs read through it.
    pub fn vfs(&self) -> &VfsRouter {
        &self.inner.router
    }

    /// The service's worker pool (additive): GUI work that needs parallelism runs on it, never on the rayon
    /// global pool (T7).
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

    /// Parses a user spec.
    pub fn parse_spec(&self, input: &OsStr) -> Result<Spec, ApiError> {
        Ok(Spec::parse_os(input, &self.spec_context())?)
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
    /// The body enters the service pool with `install` (§5.1). If the thread cannot be started, the handle's
    /// result is an `ApiError::Internal` at once.
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
                let result = run_guarded(|| service.inner.pool.install(|| f(&service, &job)));
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

    /// The stdin blob, when the adapter read one.
    pub(crate) fn stdin(&self) -> Option<&Blob> {
        self.inner.stdin.as_ref()
    }

    /// The router as a shared value.
    pub(crate) fn router(&self) -> &Arc<VfsRouter> {
        &self.inner.router
    }
}
