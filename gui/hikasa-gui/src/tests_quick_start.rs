//! M2 W13c acceptance A13.20 (D-368): the Quick Compare dialog at startup through the models. With the option off two
//! files open their view; on, binary and rules-based give `hikasa quick`'s result for identical, rules-same,
//! unimportant-only and different pairs and a missing file; `open_view_if_different`; *Open View* opens
//! `infer_view_kind`'s view; the check writes the option; `--wait` gets the quick exit code on Close and the view's on
//! Open View; IPC opens of `gui open a b` show it, `text compare --gui`, `--fv`, two folders and a session do not.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{ContentKind, ContentResult, ExitCode, JobKind};
use hikasa_session::api::{
    GuiOpenRequest, QuickMethod, QuickRequest, TextCompareRequest, ViewWords,
};
use hikasa_session::ipc::{
    CloseReason, Endpoint, IpcClient, IpcServer, Request, Response, ViewKind,
};

use crate::commands::Scope;
use crate::dialog::{ButtonRole, DialogModel};
use crate::ipc::GuiHandler;
use crate::quick_start::{OPEN_VIEW, QuickStartDialog};
use crate::tests::{TestApp, World};
use crate::tests_sync_view::find;

/// A world whose `[startup]` options are `options`.
fn world(options: &str) -> World {
    World::with_files(&[("options.toml", options)])
}

const ON: &str = "[startup]\nshow_quick_compare_dialog = true\nopen_view_if_different = false\n";

/// Opens two paths as the command line does (`resolve_open` with an inferred kind) and settles.
fn open(world: &World, app: &mut TestApp, l: &std::path::Path, r: &std::path::Path) {
    world.open(app, l, r);
    settle(world, app);
}

/// Settles until the quick dialog's job ended.
fn settle(world: &World, app: &mut TestApp) {
    let began = Instant::now();
    loop {
        world.settle(app);
        if find::<QuickStartDialog>(app).is_none_or(|d| !d.running()) {
            world.settle(app);
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `hikasa quick`'s answer for the pair.
fn quick(
    world: &World,
    l: &std::path::Path,
    r: &std::path::Path,
    method: QuickMethod,
) -> Result<(ContentResult, ExitCode), String> {
    let request = QuickRequest {
        left: l.as_os_str().to_owned(),
        right: r.as_os_str().to_owned(),
        method,
        ..QuickRequest::default()
    };
    let job = world
        .service
        .job(JobKind::QuickCompare, Arc::new(hikasa_core::NullProgress));
    world
        .service
        .quick(&request, &job)
        .map(|r| (r.outcome.result, r.exit_code))
        .map_err(|e| e.to_string())
}

#[test]
fn a13_20_with_the_option_off_two_files_open_the_view() {
    let world = world("[startup]\nshow_quick_compare_dialog = false\n");
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    open(&world, &mut app, &l, &r);
    assert!(find::<QuickStartDialog>(&mut app).is_none());
    assert_eq!(app.scope(), Scope::Text);
}

/// Each method and each kind of pair gives `quick`'s result; a missing file its error in place.
#[test]
fn a13_20_each_method_gives_the_quick_result() {
    let world = world(ON);
    let pairs = [
        ("equal", "int a;\nint b;\n", "int a;\nint b;\n"),
        ("ws", "int a;\nint b;\n", "int  a;\nint b;\n"),
        ("diff", "int a;\nint b;\n", "int a;\nint c;\n"),
    ];
    for (name, a, b) in pairs {
        let l = world.write(&format!("work/q/{name}-l.c"), a);
        let r = world.write(&format!("work/q/{name}-r.c"), b);
        for method in [QuickMethod::Binary, QuickMethod::Rules] {
            let mut app = world.app(None);
            open(&world, &mut app, &l, &r);
            let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
            if d.method != method {
                d.set_method(method);
            }
            settle(&world, &mut app);
            let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
            assert_eq!(
                d.result.clone(),
                Some(quick(&world, &l, &r, method)),
                "{name} {method:?}"
            );
            assert!(d.facts.iter().all(|f| matches!(f, Some(Ok(_)))));
            assert_eq!(d.help_topic(), "dialog-quick-compare-startup");
            // The defaults: Close when same, Open View when different.
            let default = d
                .buttons()
                .into_iter()
                .find(|b| b.default)
                .expect("a default");
            assert_eq!(default.role == OPEN_VIEW, !d.same(), "{name} {method:?}");
            // `same` is binary or rules same only: an unimportant-only result opens the view (mutation G8).
            let kind = quick(&world, &l, &r, method).expect("quick").0.kind();
            let same = matches!(kind, ContentKind::BinarySame | ContentKind::RulesSame);
            assert_eq!(d.same(), same, "{name} {method:?} {kind:?}");
        }
    }
    // Each result kind: only binary and rules same are "same" (Close is the default); unimportant differences only
    // open the view (mutation G8: no default rules make a pair unimportant-only, so the result is set here).
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    open(&world, &mut app, &l, &r);
    let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
    for (result, same) in [
        (ContentResult::BinarySame, true),
        (ContentResult::RulesSame, true),
        (ContentResult::UnimportantOnly, false),
        (ContentResult::ImportantDifferences, false),
        (ContentResult::BinaryDifferent, false),
    ] {
        let code = ExitCode::for_content(&result, false);
        d.result = Some(Ok((result.clone(), code)));
        assert_eq!(d.same(), same, "{result:?}");
        let default = d
            .buttons()
            .into_iter()
            .find(|b| b.default)
            .expect("a default");
        assert_eq!(default.role == OPEN_VIEW, !same, "{result:?}");
    }
    // A missing file: the error in place (the right side's), and Close answers a waiting caller 100.
    let l = world.write("work/q/there.c", "x\n");
    let r = world.path("work/q/missing.c");
    let (endpoint, mut app) = served(&world);
    let request = GuiOpenRequest {
        specs: vec![l.into_os_string(), r.into_os_string()],
        wait: true,
        ..GuiOpenRequest::default()
    };
    let waiter = send(&endpoint, &request, &world, true);
    arrive(&world, &mut app);
    let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
    assert!(d.result_text().is_some_and(|t| !t.is_empty()));
    assert!(matches!(d.result, Some(Err(_))), "{:?}", d.result);
    let right = d.facts[1].clone().expect("facts").expect_err("missing");
    assert!(right.starts_with("right"), "{right}");
    assert!(matches!(d.facts[0], Some(Ok(_))));
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    let (_, code) = waiter.join().expect("waiter").expect("closed");
    assert_eq!(code, ExitCode::UnknownError.code(), "Close answers 100");
}

/// `open_view_if_different` opens the view at once for a different pair and not for a same one.
#[test]
fn a13_20_open_view_if_different() {
    let world =
        world("[startup]\nshow_quick_compare_dialog = true\nopen_view_if_different = true\n");
    let (l, r) = crate::tests::text_pair(&world);
    let mut app = world.app(None);
    open(&world, &mut app, &l, &r);
    assert!(
        find::<QuickStartDialog>(&mut app).is_none(),
        "closed at once"
    );
    assert_eq!(app.scope(), Scope::Text);
    let same = world.write("work/same.c", &std::fs::read_to_string(&l).expect("left"));
    let mut app = world.app(None);
    open(&world, &mut app, &l, &same);
    assert!(
        find::<QuickStartDialog>(&mut app).is_some(),
        "a same pair keeps the dialog"
    );
    assert_eq!(app.scope(), Scope::Home);
}

/// *Open View* opens `infer_view_kind`'s view (a picture pair: Picture Compare).
#[test]
fn a13_20_open_view_opens_the_inferred_view() {
    let world = world(ON);
    let pair = crate::tests_picture_open::pair(&world);
    let mut app = world.app(None);
    open(&world, &mut app, &pair.0, &pair.1);
    let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
    assert_eq!(d.kind, ViewKind::PictureCompare);
    let w = app.win();
    app.0.dialog_button(w, OPEN_VIEW);
    world.settle(&mut app);
    assert!(find::<QuickStartDialog>(&mut app).is_none());
    assert_eq!(app.scope(), Scope::Picture);
}

/// The check writes `[startup] show_quick_compare_dialog` at once.
#[test]
fn a13_20_the_check_writes_the_option() {
    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let mut app = world.app(None);
    open(&world, &mut app, &l, &r);
    find::<QuickStartDialog>(&mut app)
        .expect("the dialog")
        .set_show_at_start(false);
    world.settle(&mut app);
    let text = std::fs::read_to_string(world.path("config/options.toml")).expect("options");
    // `false` is the default: the sparse file leaves the key out.
    assert!(!text.contains("show_quick_compare_dialog = true"), "{text}");
    assert!(!app.0.options().startup.show_quick_compare_dialog);
}

/// A server for IPC opens; returns the endpoint and the app.
fn served(world: &World) -> (Endpoint, TestApp) {
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    (endpoint, world.app(Some(handle)))
}

/// Sends `request` (resolved as `hikasa gui open` would) with `wait`, from a thread; returns its join handle.
fn send(
    endpoint: &Endpoint,
    request: &GuiOpenRequest,
    world: &World,
    wait: bool,
) -> std::thread::JoinHandle<Option<(String, u8)>> {
    let resolved = world.service.resolve_open(request).expect("resolve");
    let endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let mut client =
            IpcClient::connect(&endpoint, "test", Duration::from_secs(10)).expect("connect");
        let (id, response) = client
            .request(Request::OpenView {
                kind: resolved.kind,
                specs: resolved.specs,
                options: resolved.options,
                wait,
            })
            .expect("open");
        assert!(
            matches!(response, Response::ViewOpened { .. }),
            "{response:?}"
        );
        wait.then(|| {
            let (_, result, code) = client.wait_view_closed(id).expect("closed");
            (result.reason, code)
        })
    })
}

fn arrive(world: &World, app: &mut TestApp) {
    let began = Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    // Every caller expects the dialog: on a loaded machine the request can take more than one pass to reach it (W14:
    // one failure in a full parallel run, never alone).
    world.settle(app);
    while find::<QuickStartDialog>(app).is_none() {
        assert!(began.elapsed() < Duration::from_secs(10), "the dialog");
        std::thread::sleep(Duration::from_millis(5));
        world.settle(app);
    }
    settle(world, app);
}

/// `--wait`: Close answers the quick exit code; Open View answers the view's at its close.
#[test]
fn a13_20_waiting_callers_get_the_quick_code_or_the_views() {
    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let (endpoint, mut app) = served(&world);
    let request = GuiOpenRequest {
        specs: vec![l.clone().into_os_string(), r.clone().into_os_string()],
        wait: true,
        ..GuiOpenRequest::default()
    };
    let waiter = send(&endpoint, &request, &world, true);
    arrive(&world, &mut app);
    let expected = quick(
        &world,
        &l,
        &r,
        find::<QuickStartDialog>(&mut app).expect("dialog").method,
    )
    .expect("quick")
    .1;
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    let (_, code) = waiter.join().expect("waiter").expect("closed");
    assert_eq!(code, expected.code(), "the quick exit code");
    // Open View: the caller waits for the view.
    let waiter = send(&endpoint, &request, &world, true);
    let began = Instant::now();
    while find::<QuickStartDialog>(&mut app).is_none() {
        assert!(began.elapsed() < Duration::from_secs(10));
        world.settle(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(&world, &mut app);
    app.0.dialog_button(w, OPEN_VIEW);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    let index = app.active();
    app.close_tab(index, CloseReason::User);
    world.settle(&mut app);
    let (reason, code) = waiter.join().expect("waiter").expect("closed");
    assert_eq!(reason, "user");
    assert_eq!(code, ExitCode::RulesDifferent.code(), "the view's verdict");
}

/// IPC opens: `gui open a b` (and the bare form, the same request) shows the dialog; `text compare --gui`, `--fv`,
/// two folders do not (the mutation "the dialog shown for a forced view kind" fails here).
#[test]
fn a13_20_which_opens_show_it() {
    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let specs = vec![l.clone().into_os_string(), r.clone().into_os_string()];
    let (endpoint, mut app) = served(&world);
    let shown = |app: &mut TestApp| find::<QuickStartDialog>(app).is_some();
    // `gui open a b`.
    let plain = GuiOpenRequest {
        specs: specs.clone(),
        ..GuiOpenRequest::default()
    };
    send(&endpoint, &plain, &world, false).join().expect("sent");
    arrive(&world, &mut app);
    assert!(shown(&mut app), "gui open a b");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    // `--fv="Text Compare"`.
    let forced = GuiOpenRequest {
        kind: Some(ViewKind::TextCompare),
        ..plain.clone()
    };
    send(&endpoint, &forced, &world, false)
        .join()
        .expect("sent");
    world.settle(&mut app);
    std::thread::sleep(Duration::from_millis(50));
    world.settle(&mut app);
    assert!(!shown(&mut app), "--fv");
    assert_eq!(app.scope(), Scope::Text);
    // `text compare --gui`.
    let text = GuiOpenRequest::for_text(
        &TextCompareRequest {
            left: l.clone().into_os_string(),
            right: r.clone().into_os_string(),
            ..TextCompareRequest::default()
        },
        &ViewWords::default(),
        false,
    );
    send(&endpoint, &text, &world, false).join().expect("sent");
    std::thread::sleep(Duration::from_millis(50));
    world.settle(&mut app);
    assert!(!shown(&mut app), "text compare --gui");
    // A view word (`--display`).
    let mut worded = plain.clone();
    worded.options.display = Some("all".into());
    send(&endpoint, &worded, &world, false)
        .join()
        .expect("sent");
    std::thread::sleep(Duration::from_millis(50));
    world.settle(&mut app);
    assert!(!shown(&mut app), "a view word");
    // Two folders.
    let (a, b) = crate::tests::folder_pair(&world);
    let folders = GuiOpenRequest {
        specs: vec![a.into_os_string(), b.into_os_string()],
        ..GuiOpenRequest::default()
    };
    send(&endpoint, &folders, &world, false)
        .join()
        .expect("sent");
    std::thread::sleep(Duration::from_millis(50));
    world.settle(&mut app);
    assert!(!shown(&mut app), "two folders");
    assert_eq!(app.scope(), Scope::Folder);
}

/// Settles a little (drops and Home's Compare resolve on a job) and says whether the dialog shows.
fn shown_after(world: &World, app: &mut TestApp) -> bool {
    for _ in 0..40 {
        world.settle(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    find::<QuickStartDialog>(app).is_some()
}

/// In-window opens never show the dialog (D-368 "When": the command line only; W13c review finding 1), with the option
/// on: a drop of two files, Home's Compare, a folder row's Open and Open With ▸ Text Compare, Compare Parent Folders, a
/// named session, a workspace and a script's `load`; the command line's `hikasa a b` does.
#[test]
fn a13_20_in_window_opens_do_not_show_it() {
    use crate::app::{OpenHow, OpenTarget, UiMsg};
    use crate::commands::CommandId;
    use crate::dnd::DropTarget;
    use crate::window::{CommandArg, Invocation, OpenWithItem, ViewModel};
    use hikasa_session::sessions::{SessionName, SessionRef};

    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let mut app = world.app(None);
    let w = app.win();
    // A drop of two files on the window.
    app.0
        .drop_paths(w, DropTarget::Window, vec![l.clone(), r.clone()]);
    assert!(!shown_after(&world, &mut app), "a drop");
    assert_eq!(app.scope(), Scope::Text);
    // Home's Compare.
    let mut app = world.app(None);
    match app.active_tab_mut().and_then(|t| t.model_mut()) {
        Some(ViewModel::Home(h)) => {
            h.panel.left = l.display().to_string();
            h.panel.right = r.display().to_string();
        }
        _ => panic!("Home is the first tab"),
    }
    app.execute(CommandId::HomeCompare);
    assert!(!shown_after(&world, &mut app), "Home's Compare");
    assert_eq!(app.scope(), Scope::Text);
    // A folder view's row: Open, Open With ▸ Text Compare.
    crate::tests_folder_ops::put(&world, "l", "a.txt", "left a\n", 3_000);
    crate::tests_folder_ops::put(&world, "r", "a.txt", "right a\n", 1_000);
    let folder = crate::tests_folder_ops::open(&world, &mut app.0);
    crate::tests_folder_ops::select(&mut app.0, folder, "a.txt");
    app.execute(CommandId::Open);
    assert!(!shown_after(&world, &mut app), "a folder row's Open");
    assert_eq!(app.scope(), Scope::Text);
    app.activate(
        app.tabs()
            .iter()
            .position(|t| t.id == folder)
            .expect("the folder view"),
    );
    crate::tests_folder_ops::select(&mut app.0, folder, "a.txt");
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::TextCompare),
        ),
    );
    assert!(!shown_after(&world, &mut app), "Open With");
    assert_eq!(app.scope(), Scope::Text);
    // Compare Parent Folders of the text view.
    assert!(app.enabled(CommandId::CompareParents));
    app.execute(CommandId::CompareParents);
    assert!(!shown_after(&world, &mut app), "Compare Parent Folders");
    assert_eq!(app.scope(), Scope::Folder);
    // A named session of two files, then a workspace with it.
    crate::tests_sessions::save(&world, "pair", &crate::tests_sessions::text_session(&world));
    app.0.open_named(
        SessionRef::parse("pair").expect("ref"),
        OpenHow::Target(OpenTarget::Window(w)),
    );
    assert!(!shown_after(&world, &mut app), "a session");
    assert_eq!(app.scope(), Scope::Text);
    world.write(
        "config/workspaces/ws.toml",
        "schema_version = 1\n[[window]]\n[[window.tab]]\nsession = \"pair\"\n",
    );
    app.0
        .load_workspace(SessionName::parse("ws").expect("name"));
    assert!(!shown_after(&world, &mut app), "a workspace");
    // A script's `load` of two folders (a script opens no view).
    world.write("work/s.txt", "load l r\n");
    let claim = crate::script_dialog::ScriptClaim::take(&world.shared).expect("the slot is free");
    let mut start = crate::script_dialog::prepare(
        &world.service,
        std::ffi::OsStr::new("s.txt"),
        Vec::new(),
        None,
        false,
        None,
        None,
    )
    .expect("the script checks");
    start.claim = Some(claim);
    app.handle(UiMsg::RunScript(Box::new(start)));
    assert!(!shown_after(&world, &mut app), "a script");
    // The command line's `hikasa a b` does.
    let mut app = world.app(None);
    world.open(&mut app, &l, &r);
    assert!(shown_after(&world, &mut app), "the command line");
}

/// *Stop*, then *Close*: a waiting caller gets "no verdict" (0, reason `user`), not "load failed" (100), whether the
/// comparison still ran or had already ended unseen (W13c review finding 10); the result line says it was stopped.
#[test]
fn a13_20_stop_then_close_is_no_verdict() {
    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let (endpoint, mut app) = served(&world);
    let request = GuiOpenRequest {
        specs: vec![l.into_os_string(), r.into_os_string()],
        wait: true,
        ..GuiOpenRequest::default()
    };
    for ended in [false, true] {
        let waiter = send(&endpoint, &request, &world, true);
        let began = Instant::now();
        while world.shared.is_empty() || find::<QuickStartDialog>(&mut app).is_none() {
            assert!(began.elapsed() < Duration::from_secs(10));
            for msg in world.mailbox.drain() {
                app.handle(msg);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // No poll yet: the job is the dialog's until it takes the result.
        let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
        assert!(d.running());
        if ended {
            while !d.job_ended() {
                assert!(began.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Close);
        let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
        assert!(d.stopped && d.result.is_none(), "{:?}", d.result);
        assert_eq!(
            d.result_text().as_deref(),
            Some(crate::quick_start::STOPPED)
        );
        assert_eq!(d.buttons()[1].label, "Close");
        app.0.dialog_button(w, ButtonRole::Close);
        world.settle(&mut app);
        let (reason, code) = waiter.join().expect("waiter").expect("closed");
        assert_eq!((reason.as_str(), code), ("user", 0), "ended {ended}");
    }
}

/// `open_view_if_different` and a comparison that ended in an error (`ContentResult::Error`, a file that cannot be
/// read): the error stays in place, the view does not open (W13c review finding 11).
#[cfg(unix)]
#[test]
fn a13_20_an_error_result_stays_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let world = world(
        "[startup]\nshow_quick_compare_dialog = true\nopen_view_if_different = true\nquick_compare_method = \"binary\"\n",
    );
    let l = world.write("work/e/l.bin", "abcdef\n");
    let r = world.write("work/e/r.bin", "abcdeg\n");
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    if std::fs::read(&r).is_ok() {
        eprintln!("the file stays readable (root): nothing to show");
        return;
    }
    let mut app = world.app(None);
    open(&world, &mut app, &l, &r);
    let d = find::<QuickStartDialog>(&mut app).expect("the dialog stays");
    assert_eq!(
        d.kind_of_result(),
        Some(hikasa_core::ContentKind::Error),
        "{:?}",
        d.result
    );
    assert!(!d.opens_at_once());
    assert!(d.result_text().is_some_and(|t| !t.is_empty()));
    assert_eq!(app.scope(), Scope::Home);
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

/// The window's close button while the comparison runs: the comparison stops, the dialog closes and a waiting caller
/// is answered (no verdict); the next close ends the process. A report being written refuses the close with a notice
/// naming the report (W13c review finding 12).
#[test]
fn a13_20_the_close_button_works_at_startup() {
    let world = world(ON);
    let (l, r) = crate::tests::text_pair(&world);
    let (endpoint, mut app) = served(&world);
    let request = GuiOpenRequest {
        specs: vec![l.into_os_string(), r.into_os_string()],
        wait: true,
        ..GuiOpenRequest::default()
    };
    let waiter = send(&endpoint, &request, &world, true);
    let began = Instant::now();
    while world.shared.is_empty() || find::<QuickStartDialog>(&mut app).is_none() {
        assert!(began.elapsed() < Duration::from_secs(10));
        for msg in world.mailbox.drain() {
            app.handle(msg);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        find::<QuickStartDialog>(&mut app)
            .expect("dialog")
            .running()
    );
    let w = app.win();
    app.0.close_window(w);
    assert!(
        find::<QuickStartDialog>(&mut app).is_none(),
        "the dialog closed"
    );
    assert!(
        app.0
            .window(w)
            .expect("window")
            .notices
            .get("dialog-busy")
            .is_none()
    );
    world.settle(&mut app);
    let (reason, code) = waiter.join().expect("waiter").expect("closed");
    assert_eq!((reason.as_str(), code), ("user", 0));
    assert!(app.0.close_window(w).contains(&crate::app::Effect::Exit));
}
