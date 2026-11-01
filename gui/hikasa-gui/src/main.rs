//! `hikasa-gui`: the desktop GUI of Hikasa (ARCH-GUI-001: egui with eframe, one native window). It is a thin
//! adapter over the core crates (ARCH-L3): every comparison runs through `hikasa_session::Service`, and the
//! window only presents the results. It also hosts the IPC endpoint that `hikasa gui …` talks to
//! (ARCH-090…092).
//!
//! Startup (contract §1.10): the command line is the legacy `gui open` syntax (`normalize_legacy_args` +
//! `parse_open_args`, D-51) → logging → `Service::new` with a lazy standard-input source → `IpcServer::bind` on
//! the configuration's endpoint. When another instance holds the endpoint, the request is forwarded to it
//! (`Service::forward_open`) and this process exits 0 (ARCH-090; `--solo` runs without IPC). Otherwise the
//! server starts, the command line's view opens (`resolve_open` + `open_view_request`) and the window runs. The
//! first-run format files (`FormatStore::materialize`, D-21) are written on a background thread after the
//! window's first frame ([`startup::Deferred`]): disk flushes never delay the endpoint or the window.
//!
//! `HIKASA_GUI_BENCH=scroll`, `folder`, `two_windows`, `workspace` or `options` (D-132) runs the scripted performance mode of
//! ARCH-GUI-003 on the command line's view (always without IPC) and prints one JSON summary on stdout.
//!
//! M2 W4a: the windows (D-106), the watch thread `hikasa-watch` (D-117) and the forwarding of `hikasa::*`
//! debug events to the log book (D-116) start with the window.
//!
//! Windows: release builds use the windows subsystem, so the program never opens a console. Debug builds keep theirs
//! for the log. `hikasa <command> --help` is the way to read the help there: `hikasa-gui --help` has no console to
//! print on unless its output is redirected.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bench;
mod commands;
mod dialog;
mod dnd;
mod folder_view;
mod fonts;
mod home;
mod ipc;
mod job;
mod mailbox;
mod menus;
mod notify;
mod options;
mod session_dialog;
mod shared;
mod startup;
mod status;
mod text_view;
mod ui;
mod view_session;
mod watch;
mod window;
mod wizards;
mod workspace;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_options;
#[cfg(test)]
mod tests_options_fixes;
#[cfg(test)]
mod tests_sessions;
#[cfg(test)]
mod tests_shell;

use std::ffi::OsString;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Context as _;
use hikasa_core::ContentMethod;
use hikasa_folder::FolderDisplayFilter;
use hikasa_formats::StdProcessRunner;
use hikasa_session::api::{
    OpenArgs, SingleArg, ViewRequest, normalize_legacy_args, open_usage, parse_open_args,
};
use hikasa_session::ipc::{IpcError, IpcServer, RetryPolicy};
use hikasa_session::{
    ApiOp, Platform, Service, ServiceInit, StdinSource, SystemClock, capture_env,
    clipboard_provider,
};

use crate::app::{AppCore, Startup, UiMsg};
use crate::bench::{BenchKind, BenchState};
use crate::ipc::GuiHandler;
use crate::mailbox::Mailbox;
use crate::shared::Shared;
use crate::startup::Deferred;
use crate::ui::HikasaApp;

/// Exit code of every failure before the window runs (D-35).
const FAILURE: u8 = 100;

/// The window icon: raw RGBA, written by `packaging/generate-brand.sh` from `assets/brand/logo-light.svg`.
const WINDOW_ICON: &[u8] = include_bytes!("../../../assets/brand/window-icon-64.rgba");
/// Width and height of [`WINDOW_ICON`].
const WINDOW_ICON_SIZE: u32 = 64;

/// glibc only (D-134): its dynamic mmap threshold climbs to 32 MiB with the first large buffers freed, after which
/// such buffers stay in the arenas' free lists for good: 125 MiB of a settled 1M-line comparison, and a run-to-run
/// swing of 25 MiB. The threshold can only be fixed before the process starts (`mallopt` is `unsafe`, confined to
/// the mmap module, architecture check 1), so the program replaces itself once with the variable set. If that fails
/// it just goes on.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn fix_mmap_threshold() {
    use std::os::unix::process::CommandExt as _;
    const VARIABLE: &str = "MALLOC_MMAP_THRESHOLD_";
    if std::env::var_os(VARIABLE).is_none()
        && let Ok(exe) = std::env::current_exe()
    {
        let mut args = std::env::args_os();
        let mut command = std::process::Command::new(exe);
        if let Some(name) = args.next() {
            command.arg0(name);
        }
        let _failed = command.args(args).env(VARIABLE, "4194304").exec();
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fix_mmap_threshold();
    let process_start = Instant::now();
    let code = match run(process_start) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("hikasa-gui: error: {e:#}");
            FAILURE
        }
    };
    std::process::ExitCode::from(code)
}

/// Everything up to the window; returns the exit code when the process does not become the GUI.
fn run(process_start: Instant) -> anyhow::Result<u8> {
    let raw: Vec<OsString> = std::env::args_os().skip(1).collect();
    let args = normalize_legacy_args(&raw, Platform::HOST);
    let open = match parse_open_args(&args, Platform::HOST) {
        Ok(open) => open,
        Err(e) => {
            eprintln!("hikasa-gui: error: {e}");
            return Ok(e.exit_code(ApiOp::GuiOpen { wait: false }).code());
        }
    };
    if open.help {
        print!("{}", open_usage());
        return Ok(0);
    }
    let mailbox: Mailbox<UiMsg> = Mailbox::new();
    // `debug_messages`, kept current by the core: off until the settings are read.
    let debug = Arc::new(AtomicBool::new(false));
    init_logging(&open, &mailbox, &debug)?;
    for warning in &open.warnings {
        tracing::warn!(target: "hikasa::gui", "{}", warning.message);
    }
    let bench = match std::env::var_os("HIKASA_GUI_BENCH") {
        None => None,
        Some(v) if v == "scroll" => Some(BenchKind::Scroll),
        Some(v) if v == "folder" => Some(BenchKind::Folder),
        Some(v) if v == "two_windows" => Some(BenchKind::TwoWindows),
        Some(v) if v == "workspace" => Some(BenchKind::Workspace),
        Some(v) if v == "options" => Some(BenchKind::Options),
        Some(v) => {
            anyhow::bail!(
                "HIKASA_GUI_BENCH must be `scroll`, `folder`, `two_windows`, `workspace` or `options`, not {v:?}"
            )
        }
    };
    let env = capture_env();
    // The GUI reads its own clipboard for `clipboard://` (D-100, D-101).
    let clipboard = clipboard_provider(&env);
    let service = match Service::new(ServiceInit {
        env,
        config_dir: open.config_dir.clone(),
        threads: None,
        stdin: Some(StdinSource::new(open.stdin_name.clone(), read_stdin)),
        clock: Arc::new(SystemClock),
        runner: Arc::new(StdProcessRunner),
        options_mode: hikasa_session::settings::LoadMode::Strict,
        clipboard,
    }) {
        Ok(service) => service,
        Err(e) => {
            eprintln!("hikasa-gui: error: {e}");
            return Ok(e.exit_code(ApiOp::Other).code());
        }
    };
    for warning in service.startup_warnings() {
        tracing::warn!(target: "hikasa::gui", "{}", warning.message);
    }
    // One argument that is not an existing path may name a session or a workspace (D-88, D-127).
    let single = match open.request.specs.as_slice() {
        [arg] => service.classify_single_arg(arg),
        _ => SingleArg::Spec,
    };
    // `--silent` with a package imports headless, without a window (D-88, W2).
    if let SingleArg::Package(path) = &single
        && open.request.options.extra.contains_key("silent")
    {
        return Ok(import_silently(&service, path));
    }
    let shared = Arc::new(Shared::new());
    let server = if open.solo || bench.is_some() {
        None
    } else {
        let endpoint = match service.endpoint() {
            Ok(endpoint) => endpoint,
            Err(e) => {
                eprintln!("hikasa-gui: error: {e}");
                return Ok(e.exit_code(ApiOp::GuiOpen { wait: false }).code());
            }
        };
        match IpcServer::bind(&endpoint) {
            Ok(server) => Some(server),
            Err(IpcError::AlreadyRunning) => {
                return Ok(forward(&service, &open, &single, &RetryPolicy::default()));
            }
            Err(e @ IpcError::InsecureEndpoint { .. }) => {
                eprintln!("hikasa-gui: error: {e}");
                return Ok(FAILURE);
            }
            Err(e) => {
                tracing::warn!(
                    target: "hikasa::gui",
                    "the IPC endpoint {} is not available ({e}); running without it",
                    endpoint.display()
                );
                None
            }
        }
    };
    // The command line's own view (resolved against this process's current folder).
    let named = !matches!(single, SingleArg::Spec);
    let initial = if open.request.specs.is_empty() || named {
        None
    } else {
        let resolved = service.resolve_open(&open.request).and_then(|r| {
            service
                .open_view_request(r.kind, &r.specs, &r.options)
                .map(|view| (r, view))
        });
        match resolved {
            Ok(initial) => Some(initial),
            Err(e) => {
                eprintln!("hikasa-gui: error: {e}");
                return Ok(e.exit_code(ApiOp::GuiOpen { wait: false }).code());
            }
        }
    };
    let handle = match server {
        Some(server) => {
            let handler = GuiHandler::new(service.clone(), Arc::clone(&shared), mailbox.clone());
            match server.spawn(Arc::new(handler)) {
                Ok(handle) => Some(handle),
                Err(e) => {
                    tracing::warn!(target: "hikasa::gui", "cannot start the IPC server: {e}");
                    None
                }
            }
        }
        None => None,
    };
    // The watch starts from the stamps of now (D-117); the bench runs without it.
    let watch = bench
        .is_none()
        .then(|| crate::watch::Watcher::new(service.clone()));
    // Written after the first frame, off the UI thread: never on the way to the endpoint or the window.
    let deferred =
        Deferred::materialize_formats(&service, watch.as_ref().map(|(_, link)| link.clone()));
    let mut app = AppCore::new(
        service,
        shared,
        mailbox.clone(),
        handle,
        Arc::new(fonts::SystemFonts),
    );
    app.set_debug_flag(debug);
    let mut bench_state = bench.map(|kind| BenchState::new(kind, process_start));
    if bench.is_some_and(|b| b != BenchKind::Options) && initial.is_none() && !named {
        anyhow::bail!("the benchmark mode needs two specs");
    }
    // The startup workspace and the command line's session open after the first frame (§1.4.12).
    match &single {
        SingleArg::Session(reference) => app.set_startup(Startup::Session(reference.clone())),
        SingleArg::Workspace(name) => app.set_startup(Startup::Workspace(name.to_string())),
        SingleArg::Package(path) => app.set_startup(Startup::Import(path.clone())),
        _ if initial.is_none() => app.set_startup(Startup::Options),
        _ => {}
    }
    if let Some((resolved, mut request)) = initial {
        if bench == Some(BenchKind::Folder)
            && let ViewRequest::Folder {
                request: folder,
                display,
                expand_all,
            } = &mut request
        {
            // The heaviest incremental case: every folder opens as it arrives, every item is shown, and CRC
            // content tests keep the scan running while the tree is drawn.
            *expand_all = true;
            display.filter = FolderDisplayFilter::All;
            folder.patch.content_method = Some(ContentMethod::Crc);
        }
        let target = app.open_target();
        let view = app.open_local(
            resolved.kind,
            request,
            &resolved.options,
            resolved.specs,
            target,
        );
        if let Some(b) = bench_state.as_mut() {
            b.set_view(view);
        }
    }
    launch(app, mailbox, bench_state, deferred, watch)
}

/// A second instance (ARCH-090): the request goes to the running GUI; exit 0 or the error's code. Without specs
/// there is nothing to open: the running instance must answer.
///
/// A session or a workspace goes as `LoadSession` / `LoadWorkspace`, a settings package as `ImportPackage` (D-127).
fn forward(service: &Service, open: &OpenArgs, single: &SingleArg, retry: &RetryPolicy) -> u8 {
    let result = if let SingleArg::Session(reference) = single {
        service
            .gui_load_session(&reference.to_string(), false, None, retry)
            .map(|_| ())
            .map_err(|e| (e, ApiOp::GuiOpen { wait: false }))
    } else if let SingleArg::Workspace(name) = single {
        service
            .gui_load_workspace(name.as_str(), None, retry)
            .map(|_| ())
            .map_err(|e| (e, ApiOp::GuiOpen { wait: false }))
    } else if let SingleArg::Package(path) = single {
        service
            .gui_import_package(path, None, retry)
            .map_err(|e| (e, ApiOp::GuiOpen { wait: false }))
    } else if open.request.specs.is_empty() {
        service
            .gui_ping()
            .map(|_| ())
            .map_err(|e| (e, ApiOp::GuiControl))
    } else {
        service
            .forward_open(&open.request)
            .map(|_| ())
            .map_err(|e| (e, ApiOp::GuiOpen { wait: false }))
    };
    match result {
        Ok(()) => 0,
        Err((e, op)) => {
            eprintln!("hikasa-gui: error: {e}");
            e.exit_code(op).code()
        }
    }
}

/// `hikasa-gui <file.hikasa-pkg> --silent` (D-88): every category and item, no window; exit 0, or 100 with a failure.
fn import_silently(service: &Service, path: &std::path::Path) -> u8 {
    let choices = hikasa_session::package::ImportChoices::default();
    match service.settings_import(path, &choices, false) {
        Ok(dto) if dto.ok() => 0,
        Ok(dto) => {
            for r in dto.results.iter().filter(|r| !r.ok) {
                eprintln!(
                    "hikasa-gui: error: {}: {}",
                    r.name,
                    r.error.as_deref().unwrap_or("failed")
                );
            }
            FAILURE
        }
        Err(e) => {
            eprintln!("hikasa-gui: error: {e}");
            FAILURE
        }
    }
}

fn read_stdin() -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Forwards `hikasa::*` events at debug level and above to the log book (D-116), only while `[tweaks.log_panel]
/// debug_messages` is on (the flag the core keeps current): otherwise nothing is formatted, posted or woken.
struct LogForward {
    mailbox: Mailbox<UiMsg>,
    debug: Arc<AtomicBool>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogForward {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if !self.debug.load(Ordering::Relaxed) {
            return;
        }
        struct Text(String);
        impl tracing::field::Visit for Text {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                } else {
                    self.0.push_str(&format!(" {}={value:?}", field.name()));
                }
            }
        }
        let mut text = Text(String::new());
        event.record(&mut text);
        let meta = event.metadata();
        self.mailbox.post(UiMsg::Debug(format!(
            "{} {}: {}",
            meta.level(),
            meta.target(),
            text.0
        )));
    }
}

/// Logging (`--log`, `--verbose`): warnings on stderr, the log file at INFO (DEBUG with `--verbose`), and the
/// `hikasa::*` debug events to the log book.
fn init_logging(
    open: &OpenArgs,
    mailbox: &Mailbox<UiMsg>,
    debug: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let level = if open.verbose {
        LevelFilter::DEBUG
    } else {
        LevelFilter::WARN
    };
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_filter(Targets::new().with_default(level));
    let file_layer = match &open.log {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("cannot open the log file {}", path.display()))?;
            let file_level = if open.verbose {
                LevelFilter::DEBUG
            } else {
                LevelFilter::INFO
            };
            Some(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::sync::Mutex::new(file))
                    .with_ansi(false)
                    .with_filter(Targets::new().with_default(file_level)),
            )
        }
        None => None,
    };
    let book_layer = LogForward {
        mailbox: mailbox.clone(),
        debug: Arc::clone(debug),
    }
    .with_filter(
        Targets::new()
            .with_target("hikasa", LevelFilter::DEBUG)
            .with_default(LevelFilter::OFF),
    );
    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .with(book_layer)
        .try_init()
        .context("cannot install the logger")
}

/// Runs the window until it closes; the process then exits 0 (the views reported their outcomes on the way).
/// `deferred` starts after the first frame.
fn launch(
    app: AppCore,
    mailbox: Mailbox<UiMsg>,
    bench: Option<BenchState>,
    deferred: Deferred,
    watch: Option<(crate::watch::Watcher, crate::watch::WatchLink)>,
) -> anyhow::Result<u8> {
    // The benchmark measures a large window (about a maximised full-HD laptop window); otherwise the size of the
    // last window closed (`state.toml [window]`, D-108).
    let root = app.windows()[0].geometry.clone();
    let size = if bench.is_some() {
        [1920.0, 1200.0]
    } else {
        [root.width as f32, root.height as f32]
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hikasa")
            .with_app_id("hikasa")
            .with_icon(egui::IconData {
                rgba: WINDOW_ICON.to_vec(),
                width: WINDOW_ICON_SIZE,
                height: WINDOW_ICON_SIZE,
            })
            .with_inner_size(size)
            .with_min_inner_size([640.0, 400.0])
            .with_maximized(bench.is_none() && root.maximized),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Hikasa",
        options,
        Box::new(move |cc| {
            Ok(Box::new(HikasaApp::new(
                cc, app, mailbox, bench, deferred, watch,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("cannot open the window: {e}"))?;
    Ok(0)
}
