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
//! `HIKASA_GUI_BENCH=scroll` or `folder` runs the scripted performance mode of ARCH-GUI-003 on the command
//! line's view (always without IPC) and prints one JSON summary on stdout.

mod app;
mod bench;
mod commands;
mod folder_view;
mod home;
mod ipc;
mod job;
mod mailbox;
mod shared;
mod startup;
mod text_view;
mod ui;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use hikasa_core::ContentMethod;
use hikasa_folder::FolderDisplayFilter;
use hikasa_formats::StdProcessRunner;
use hikasa_session::api::{
    OpenArgs, ViewRequest, normalize_legacy_args, open_usage, parse_open_args,
};
use hikasa_session::ipc::{IpcError, IpcServer};
use hikasa_session::{
    ApiOp, Platform, Service, ServiceInit, StdinSource, SystemClock, capture_env,
    clipboard_provider,
};

use crate::app::{AppCore, UiMsg};
use crate::bench::{BenchKind, BenchState};
use crate::ipc::GuiHandler;
use crate::mailbox::Mailbox;
use crate::shared::Shared;
use crate::startup::Deferred;
use crate::ui::HikasaApp;

/// Exit code of every failure before the window runs (D-35).
const FAILURE: u8 = 100;

fn main() -> std::process::ExitCode {
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
    init_logging(&open)?;
    for warning in &open.warnings {
        tracing::warn!(target: "hikasa::gui", "{}", warning.message);
    }
    let bench = match std::env::var_os("HIKASA_GUI_BENCH") {
        None => None,
        Some(v) if v == "scroll" => Some(BenchKind::Scroll),
        Some(v) if v == "folder" => Some(BenchKind::Folder),
        Some(v) => anyhow::bail!("HIKASA_GUI_BENCH must be `scroll` or `folder`, not {v:?}"),
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
    let shared = Arc::new(Shared::new());
    let mailbox: Mailbox<UiMsg> = Mailbox::new();
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
            Err(IpcError::AlreadyRunning) => return Ok(forward(&service, &open)),
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
    let initial = if open.request.specs.is_empty() {
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
    // Written after the first frame, off the UI thread: never on the way to the endpoint or the window.
    let deferred = Deferred::materialize_formats(&service);
    let mut app = AppCore::new(service, shared, mailbox.clone(), handle);
    let mut bench_state = bench.map(|kind| BenchState::new(kind, process_start));
    if bench.is_some() && initial.is_none() {
        anyhow::bail!("the benchmark mode needs two specs");
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
        let view = app.open_local(
            resolved.kind,
            request,
            &resolved.options,
            resolved.specs,
            None,
        );
        if let Some(b) = bench_state.as_mut() {
            b.set_view(view);
        }
    }
    launch(app, mailbox, bench_state, deferred)
}

/// A second instance (ARCH-090): the request goes to the running GUI; exit 0 or the error's code. Without specs
/// there is nothing to open: the running instance must answer.
fn forward(service: &Service, open: &OpenArgs) -> u8 {
    let result = if open.request.specs.is_empty() {
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

fn read_stdin() -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Logging (`--log`, `--verbose`): warnings on stderr, the log file at INFO (DEBUG with `--verbose`).
fn init_logging(open: &OpenArgs) -> anyhow::Result<()> {
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
    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
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
) -> anyhow::Result<u8> {
    // The benchmark measures a large window (about a maximised full-HD laptop window).
    let size = if bench.is_some() {
        [1920.0, 1200.0]
    } else {
        [1280.0, 820.0]
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hikasa")
            .with_app_id("hikasa")
            .with_inner_size(size)
            .with_min_inner_size([640.0, 400.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Hikasa",
        options,
        Box::new(move |cc| Ok(Box::new(HikasaApp::new(cc, app, mailbox, bench, deferred)))),
    )
    .map_err(|e| anyhow::anyhow!("cannot open the window: {e}"))?;
    Ok(0)
}
