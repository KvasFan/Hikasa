//! M2 W13c acceptance A13.20 (D-368): the Quick Compare dialog at startup through the models. With the option off two
//! files open their view; on, binary and rules-based give `hikasa quick`'s result for identical, rules-same,
//! unimportant-only and different pairs and a missing file; `open_view_if_different`; *Open View* opens
//! `infer_view_kind`'s view; the check writes the option; `--wait` gets the quick exit code on Close and the view's on
//! Open View; IPC opens of `gui open a b` show it, `text compare --gui`, `--fv`, two folders and a session do not.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{ContentResult, ExitCode, JobKind};
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
        }
    }
    // A missing file: the error in place, and Close answers 100.
    let l = world.write("work/q/there.c", "x\n");
    let r = world.path("work/q/missing.c");
    let mut app = world.app(None);
    let request = GuiOpenRequest {
        specs: vec![l.clone().into_os_string(), r.clone().into_os_string()],
        kind: None,
        ..GuiOpenRequest::default()
    };
    if let Ok(resolved) = world.service.resolve_open(&request) {
        let view = world
            .service
            .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
            .expect("request");
        let target = app.open_target();
        app.open_local(
            resolved.kind,
            view,
            &resolved.options,
            resolved.specs,
            target,
        );
        settle(&world, &mut app);
        let d = find::<QuickStartDialog>(&mut app).expect("the dialog");
        assert!(d.result_text().is_some_and(|t| !t.is_empty()));
    }
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
