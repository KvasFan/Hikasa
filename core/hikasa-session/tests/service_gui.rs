//! `Service::gui_open`, `gui_ping`, `gui_list`, `gui_close`, `gui_quit` against a scripted GUI on a per-test
//! endpoint (the `HIKASA_IPC_ENDPOINT` override in `EnvFacts`; §3.6, D-22, D-23, D-49, D-50).

mod common;

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use common::{CloseMode, FakeGui, FakeLauncher, os, write};
use hikasa_core::ExitCode;
use hikasa_session::api::{ApiOp, GuiOpenOutcome, GuiOpenRequest};
use hikasa_session::ipc::{
    IpcError, IpcServer, OpenOptions, Request, RetryPolicy, ViewId, ViewKind,
};
use hikasa_session::{ApiError, Service};

fn start(service: &Service, gui: Arc<FakeGui>) -> hikasa_session::ipc::ServerHandle {
    IpcServer::bind(&service.endpoint().unwrap())
        .unwrap()
        .spawn(gui)
        .unwrap()
}

fn open(specs: &[OsString], wait: bool) -> GuiOpenRequest {
    GuiOpenRequest {
        specs: specs.to_vec(),
        kind: None,
        options: OpenOptions::default(),
        wait,
    }
}

fn sent_open_view(gui: &FakeGui) -> (ViewKind, Vec<String>, OpenOptions, bool) {
    let requests = gui.requests.lock();
    requests
        .iter()
        .rev()
        .find_map(|r| match r {
            Request::OpenView {
                kind,
                specs,
                options,
                wait,
            } => Some((*kind, specs.clone(), options.clone(), *wait)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn no_gui_running() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let ping = service.gui_ping().unwrap_err();
    assert!(matches!(ping, ApiError::Ipc(IpcError::NotRunning)));
    assert_eq!(ping.exit_code(ApiOp::GuiControl), ExitCode::UnknownError);
    for err in [
        service.gui_list().unwrap_err(),
        service.gui_close(ViewId(1)).unwrap_err(),
        service.gui_quit().unwrap_err(),
    ] {
        assert_eq!(err.exit_code(ApiOp::GuiControl), ExitCode::UnknownError);
    }
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let specs = [os(&a), os(&a)];
    // --no-launch: 100 without --wait, 102 with it (D-23).
    let err = service.gui_open(&open(&specs, false), None).unwrap_err();
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
    let err = service.gui_open(&open(&specs, true), None).unwrap_err();
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: true }),
        ExitCode::CannotWaitForGui
    );
    // No executable: 103.
    let missing = FakeLauncher {
        not_found: true,
        ..FakeLauncher::default()
    };
    let err = service
        .gui_open(&open(&specs, true), Some(&missing))
        .unwrap_err();
    assert!(matches!(err, ApiError::GuiNotFound));
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: true }),
        ExitCode::GuiNotFound
    );
    // A launched GUI that never listens: 102 (with a short budget).
    let never = FakeLauncher::default();
    let policy = RetryPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(20),
        total: Duration::from_millis(60),
    };
    let err = service
        .gui_open_with(&open(&specs, false), Some(&never), &policy)
        .unwrap_err();
    assert!(matches!(err, ApiError::Ipc(IpcError::LaunchTimeout)));
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::CannotWaitForGui
    );
}

#[test]
fn open_wait_returns_the_view_exit_code() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Similar);
    let server = start(&service, Arc::clone(&gui));
    let w = tmp.path().join("work");
    write(&w, "a.c", b"int a;\n");
    write(&w, "b.c", b"int b;\n");
    // Relative specs are resolved with the captured cwd before sending (the GUI has another cwd).
    let mut req = open(&[OsString::from("a.c"), OsString::from("b.c")], true);
    req.options.text_rules_file = Some("rules.toml".into());
    req.options.titles[0] = Some("Mine".into());
    let report = service
        .gui_open_with(&req, None, &RetryPolicy::default())
        .unwrap();
    match &report.outcome {
        GuiOpenOutcome::Closed {
            exit_code, result, ..
        } => {
            assert_eq!(*exit_code, 12);
            assert_eq!(result.reason, "user");
        }
        other => panic!("{other:?}"),
    }
    let (kind, specs, options, wait) = sent_open_view(&gui);
    assert_eq!(kind, ViewKind::TextCompare);
    assert!(wait);
    assert_eq!(
        specs,
        vec![
            w.join("a.c").to_string_lossy().into_owned(),
            w.join("b.c").to_string_lossy().into_owned()
        ]
    );
    assert_eq!(report.specs, specs);
    assert_eq!(
        options.text_rules_file,
        Some(w.join("rules.toml").to_string_lossy().into_owned()),
        "rules file paths are made absolute (D-50)"
    );
    assert_eq!(options.titles[0].as_deref(), Some("Mine"));
    let dto = serde_json::to_value(report.dto()).unwrap();
    assert_eq!(dto["waited"], true);
    assert_eq!(dto["view_exit_code"], 12);
    assert_eq!(dto["spooled_stdin"], serde_json::Value::Null);

    // Folders → FolderCompare, without waiting.
    std::fs::create_dir_all(w.join("d1")).unwrap();
    std::fs::create_dir_all(w.join("d2")).unwrap();
    let outcome = service
        .gui_open(&open(&[os(&w.join("d1")), os(&w.join("d2"))], false), None)
        .unwrap();
    assert!(matches!(outcome, GuiOpenOutcome::Opened { .. }));
    assert_eq!(sent_open_view(&gui).0, ViewKind::FolderCompare);
    // --fv forces the kind.
    let mut forced = open(&[os(&w.join("a.c")), os(&w.join("b.c"))], false);
    forced.kind = Some(ViewKind::FolderCompare);
    service.gui_open(&forced, None).unwrap();
    assert_eq!(sent_open_view(&gui).0, ViewKind::FolderCompare);
    // A kind the M1 GUI does not serve is refused by the server (exit 100).
    forced.kind = Some(ViewKind::TextMerge);
    let err = service.gui_open(&forced, None).unwrap_err();
    assert!(
        matches!(&err, ApiError::Ipc(IpcError::Remote { code, .. }) if code == "unsupported_view")
    );
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
    server.shutdown().unwrap();
}

#[test]
fn stdin_is_spooled_and_deleted_after_wait() {
    let tmp = common::short_tempdir();
    let stdin = common::blob(b"print(1)\n", Some("../evil/x.py"));
    let service = common::service_with(
        tmp.path(),
        Some(stdin),
        Arc::new(common::UpperRunner::default()),
    );
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesDifferent);
    let server = start(&service, Arc::clone(&gui));
    let w = tmp.path().join("work");
    let b = write(&w, "b.py", b"print(2)\n");
    let report = service
        .gui_open_with(
            &open(&[OsString::from("-"), os(&b)], true),
            None,
            &RetryPolicy::default(),
        )
        .unwrap();
    let spooled = report.spooled_stdin.clone().unwrap();
    let cache = service.config().cache_dir.join("stdin");
    assert_eq!(
        spooled,
        cache.join("1").join("x.py"),
        "the name hint's file name only"
    );
    let (_, specs, _, _) = sent_open_view(&gui);
    assert_eq!(specs[0], spooled.to_string_lossy());
    assert!(!spooled.exists(), "deleted after --wait");
    assert!(!spooled.parent().unwrap().exists());
    assert!(matches!(
        report.outcome,
        GuiOpenOutcome::Closed { exit_code: 13, .. }
    ));

    // Without --wait the file stays for the GUI to read; concurrent spools never share a file.
    let first = service
        .gui_open_with(
            &open(&[OsString::from("-"), os(&b)], false),
            None,
            &RetryPolicy::default(),
        )
        .unwrap()
        .spooled_stdin
        .unwrap();
    let second = service
        .gui_open_with(
            &open(&[OsString::from("-"), os(&b)], false),
            None,
            &RetryPolicy::default(),
        )
        .unwrap()
        .spooled_stdin
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), b"print(1)\n");
    assert!(second.exists());
    server.shutdown().unwrap();

    // When the view cannot be opened the spool is removed again.
    let gone = service
        .gui_open(&open(&[OsString::from("-"), os(&b)], false), None)
        .unwrap_err();
    assert!(matches!(gone, ApiError::Ipc(IpcError::NotRunning)));
    let leftovers: Vec<_> = std::fs::read_dir(&cache).unwrap().flatten().collect();
    assert_eq!(leftovers.len(), 2, "only the two un-waited spools remain");
    // `-` twice is refused.
    assert!(matches!(
        service.gui_open(
            &open(&[OsString::from("-"), OsString::from("-")], false),
            None
        ),
        Err(ApiError::InvalidArgument(_))
    ));
}

#[test]
fn clipboard_and_missing_stdin_are_rejected() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let err = service
        .gui_open(
            &open(&[OsString::from("clipboard://"), os(&a)], false),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)));
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
    let err = service
        .gui_open(&open(&[OsString::from("-"), os(&a)], false), None)
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "no stdin blob");
    let err = service.gui_open(&open(&[os(&a)], false), None).unwrap_err();
    assert!(
        matches!(err, ApiError::InvalidArgument(_)),
        "one spec cannot be inferred"
    );
}

#[test]
fn control_requests() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = start(&service, Arc::clone(&gui));
    let info = service.gui_ping().unwrap();
    assert_eq!(info.server, "fake-gui 0.0.0");
    assert_eq!(info.version, 1);
    let ping =
        hikasa_session::api::dto::GuiPingDto::new(service.endpoint().unwrap().display(), &info);
    assert_eq!(ping.protocol_version, 1);
    assert!(service.gui_list().unwrap().is_empty());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let GuiOpenOutcome::Opened { view } = service
        .gui_open(&open(&[os(&a), os(&a)], false), None)
        .unwrap()
    else {
        panic!("not waiting")
    };
    let views = service.gui_list().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].view, view);
    service.gui_close(view).unwrap();
    let err = service.gui_close(view).unwrap_err();
    assert!(matches!(&err, ApiError::Ipc(IpcError::Remote { code, .. }) if code == "not_found"));
    assert_eq!(err.exit_code(ApiOp::GuiControl), ExitCode::UnknownError);
    service.gui_quit().unwrap();
    server.shutdown().unwrap();
}

#[test]
fn launching_reaches_the_gui_that_comes_up() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let endpoint = service.endpoint().unwrap();
    let holder = Arc::new(parking_lot::Mutex::new(None));
    let slot = Arc::clone(&holder);
    let launcher = FakeLauncher {
        on_sleep: parking_lot::Mutex::new(Some(Box::new(move || {
            let server = IpcServer::bind(&endpoint)
                .unwrap()
                .spawn(FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame))
                .unwrap();
            *slot.lock() = Some(server);
        }))),
        ..FakeLauncher::default()
    };
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let outcome = service
        .gui_open(&open(&[os(&a), os(&a)], true), Some(&launcher))
        .unwrap();
    assert!(matches!(
        outcome,
        GuiOpenOutcome::Closed { exit_code: 2, .. }
    ));
    assert_eq!(
        launcher.launches.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    if let Some(server) = holder.lock().take() {
        server.shutdown().unwrap();
    }
}
