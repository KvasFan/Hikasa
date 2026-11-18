//! A13.21 (M2 W13a, D-369) in the window: `hex_compare` is served over IPC (`hex compare --gui`'s request, its words
//! applied in the view) and by `--fv`; the `--wait` outcomes of a hex view (1, 11, 2, 13; `NoVerdict`, `LoadFailed`);
//! a folder view's Open of a Hex-format pair opens a child Hex Compare view; Open With ▸ Hex Compare; Home's type and
//! New Session ▸ Hex Compare; `TabDuplicate` with the view state; a workspace restores a hex tab lazily.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::ExitCode;
use hikasa_session::api::{GuiOpenRequest, HexCompareRequest, HexLayoutWords, HexRuleWords};
use hikasa_session::ipc::{
    CloseReason, Endpoint, IpcClient, IpcServer, Request, Response, ViewKind,
};
use hikasa_session::sessions::SessionType;

use crate::commands::{CommandId, Scope};
use crate::ipc::GuiHandler;
use crate::tests::World;
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, write_pair};
use crate::window::{CommandArg, Invocation, OpenWithItem};

/// A13.21: `hex compare --gui`'s request is served (no `unsupported_view`), its rule and layout words reach the view;
/// the waiting client gets the view's verdict at close (D-331's codes).
#[test]
fn a13_21_hex_compare_gui_is_served_with_its_words_and_wait() {
    let world = hex_world();
    let a = bytes(111, 4000);
    let mut b = a.clone();
    b[10] ^= 1;
    let (pa, pb) = write_pair(&world, &a, &b);
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = world.app(Some(handle));
    let timeout = Duration::from_secs(10);
    let request = HexCompareRequest {
        left: pa.clone().into_os_string(),
        right: pb.clone().into_os_string(),
        rules: HexRuleWords {
            alignment: Some("complete".into()),
            ..HexRuleWords::default()
        },
        layout: HexLayoutWords {
            bytes_per_line: Some("8".into()),
            display: Some("all".into()),
            ..HexLayoutWords::default()
        },
        ..HexCompareRequest::default()
    };
    world.service.check_hex_words(&request).expect("words");
    let open = GuiOpenRequest::for_hex(&request, true);
    let resolved = world.service.resolve_open(&open).expect("resolve");
    assert_eq!(resolved.kind, ViewKind::HexCompare);
    let wait_endpoint = endpoint.clone();
    let waiting = std::thread::spawn(move || {
        let mut client = IpcClient::connect(&wait_endpoint, "test", timeout).expect("connect");
        let (id, response) = client
            .request(Request::OpenView {
                kind: resolved.kind,
                specs: resolved.specs,
                options: resolved.options,
                wait: true,
            })
            .expect("open");
        let Response::ViewOpened { view } = response else {
            panic!("{response:?}");
        };
        (view, client.wait_view_closed(id).expect("closed"))
    });
    let began = Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(Duration::from_millis(5));
    }
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex);
    let view = app.active_tab().expect("tab").id;
    let m = hex_model(&mut app, view);
    assert_eq!(m.display().layout.bytes_per_line, 8);
    assert_eq!(m.display().filter, hikasa_hex::DisplayFilter::All);
    assert_eq!(
        m.alignment().expect("landed").mode,
        hikasa_hex::AlignMode::Complete
    );
    let index = app.active();
    app.close_tab(index, CloseReason::User);
    world.settle(&mut app);
    let (opened, (closed, result, code)) = waiting.join().expect("waiter");
    assert_eq!((opened, closed), (view, view));
    assert_eq!(
        code,
        ExitCode::BinaryDifferent.code(),
        "binary_different: 11"
    );
    assert_eq!(result.verdict.as_deref(), Some("binary_different"));
}

/// A converter that writes the upper-cased source (`argv = [prog, %s, %t]`).
struct Upper;

impl hikasa_formats::ProcessRunner for Upper {
    fn run(
        &self,
        argv: &[std::ffi::OsString],
        _timeout: Option<Duration>,
        cancel: &hikasa_core::CancelToken,
    ) -> Result<i32, hikasa_formats::ConvertError> {
        cancel.check()?;
        let source = std::fs::read(&argv[1])?;
        std::fs::write(&argv[2], source.to_ascii_uppercase())?;
        Ok(0)
    }
}

/// A13.21: the outcomes at close (D-331): identical 1, different 11; a converted pair: raw identical 1, outputs
/// identical 2, different 13; a view without a landed alignment `NoVerdict` (0) after its 1 s wait.
#[test]
fn a13_21_outcomes_at_close() {
    let world = World::with_files_and_init(
        &[
            ("formats/firmware.toml", crate::tests_hex_open::HEX_FORMAT),
            (
                "formats/hexup.toml",
                "schema_version = 1\nid = \"hexup\"\nname = \"Hex Upper\"\nkind = \"hex\"\nmask = \"*.up\"\n\n[conversion]\nmethod = \"external\"\nload = \"conv %s %t\"\n",
            ),
        ],
        |init| init.runner = Arc::new(Upper),
    );
    let mut app = world.app(None);
    let code = |app: &mut crate::tests::TestApp, view| {
        hex_model(app, view)
            .outcome(&world.service)
            .close(CloseReason::User, false)
            .1
    };
    let a = bytes(112, 2000);
    let mut b = a.clone();
    b[5] ^= 1;
    let view = open_pair(&world, &mut app, &a, &a);
    assert_eq!(code(&mut app, view), ExitCode::BinarySame);
    let view = open_pair(&world, &mut app, &a, &b);
    assert_eq!(code(&mut app, view), ExitCode::BinaryDifferent);
    let up = |name: &str, text: &str| world.write(&format!("work/u/{name}"), text);
    for (l, r, want) in [
        ("abc", "abc", ExitCode::BinarySame),
        ("abc", "ABC", ExitCode::RulesSame),
        ("abc", "abd", ExitCode::RulesDifferent),
    ] {
        let (pl, pr) = (up("one.up", l), up("two.up", r));
        let view = world.open(&mut app, &pl, &pr);
        world.settle(&mut app);
        assert_eq!(app.scope(), Scope::Hex);
        assert_eq!(code(&mut app, view), want, "{l} {r}");
    }
    // Held: no verdict within 1 s of the close.
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = crate::tests_hex_open::Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.reload();
    crate::tests_hex_open::until(&world, &mut app, |_| {
        gate.reached.load(std::sync::atomic::Ordering::SeqCst) == 1
    });
    assert_eq!(code(&mut app, view), ExitCode::Success);
    let _ = Path::new("");
}

/// A13.21: a folder view's Open of a Hex-format pair opens a Hex Compare child view; Open With ▸ Hex Compare opens two
/// selected files of any format.
#[test]
fn a13_21_folder_open_and_open_with() {
    let world = hex_world();
    let mut app = world.app(None);
    let l = world.path("work/l");
    let r = world.path("work/r");
    std::fs::create_dir_all(&l).expect("mkdir");
    std::fs::create_dir_all(&r).expect("mkdir");
    std::fs::write(l.join("x.bin"), bytes(113, 500)).expect("w");
    std::fs::write(r.join("x.bin"), bytes(114, 501)).expect("w");
    std::fs::write(l.join("y.txt"), "a\n").expect("w");
    std::fs::write(r.join("y.txt"), "bb\n").expect("w");
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let folder = app.active_tab().expect("tab").id;
    crate::tests_folder_ops::select(&mut app, folder, "x.bin");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(
        app.scope(),
        Scope::Hex,
        "a Hex-format pair opens in Hex Compare"
    );
    // Open With ▸ Hex Compare of the text pair.
    let index = app
        .tabs()
        .iter()
        .position(|t| t.id == folder)
        .expect("folder");
    app.activate(index);
    crate::tests_folder_ops::select(&mut app, folder, "y.txt");
    app.execute(Invocation::with(
        CommandId::OpenWithList,
        CommandArg::OpenWith(OpenWithItem::HexCompare),
    ));
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex);
    let view = app.active_tab().expect("tab").id;
    assert_eq!(
        hex_model(&mut app, view).inputs()[0],
        l.join("y.txt").display().to_string()
    );
}

/// A13.21: Home's and New Session's types list Hex Compare; Home's Compare with that type opens any two files in Hex
/// Compare; `TabDuplicate` keeps the view state; a workspace restores the hex tab lazily.
#[test]
fn a13_21_home_duplicate_and_workspace() {
    assert!(
        crate::menus::OPENABLE_TYPES
            .iter()
            .any(|(k, c)| *k == SessionType::HexCompare && *c == "Hex Compare")
    );
    let world = hex_world();
    let mut app = world.app(None);
    let x = world.write("work/x.dat", "plain\n");
    let y = world.write("work/y.dat", "plane\n");
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::SessionNewList,
            CommandArg::SessionType(SessionType::HexCompare),
        ),
    );
    world.settle(&mut app);
    if let Some(crate::app::ViewModel::Home(h)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        h.panel.left = x.display().to_string();
        h.panel.right = y.display().to_string();
    }
    app.execute(CommandId::HomeCompare);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex, "any two files open in Hex Compare");
    let view = app.active_tab().expect("tab").id;
    let m = hex_model(&mut app, view);
    let mut layout = m.display().layout;
    layout.bytes_per_line = 12;
    m.set_layout(layout, true, crate::hex::PaneLayout::OverUnder, 2);
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let dup = app.active_tab().expect("tab").id;
    assert_ne!(dup, view);
    let d = *hex_model(&mut app, dup).display();
    assert_eq!(d.layout.bytes_per_line, 12);
    assert_eq!(d.context, 2);
    // A workspace with the hex tab: restored lazily (a pending tab until shown).
    crate::tests_sessions::save_workspace(&world, &mut app, "ws");
    world.settle(&mut app);
    let mut app2 = world.app(None);
    app2.0
        .load_workspace(hikasa_session::sessions::SessionName::parse("ws").expect("name"));
    world.settle(&mut app2);
    assert!(app2.tabs().iter().any(|t| t.scope() == Scope::Hex));
}
