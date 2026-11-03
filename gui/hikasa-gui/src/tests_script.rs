//! A8.19 (§1.8.14, D-229): the status window and the Run Script dialog through their models, with real runs on the
//! test's folders (the Service's cwd is `work/`). The run's events render the command line, the progress, the log lines
//! (capped), the end line; Stop cancels the run's job; W6's questions are confirmation dialogs stacked on the status
//! window and their answers reach the run; the stop-on-error question; `close_when_finished`, `beep_when_finished` and
//! `--closescript`; the end notice and the log panel entries; a second `RunScript` while one runs is `busy`; the
//! catalogue command `RunScript`, its dialog's arguments and check errors; Esc.

use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_session::api::script::LineKind;
use hikasa_session::ipc::{IpcHandler, Request, Response};
use hikasa_session::log::LogKind;
use hikasa_session::settings::Shortcut;

use crate::app::UiMsg;
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel as _};
use crate::ipc::GuiHandler;
use crate::job::Wake;
use crate::notify::{NoticeAction, NoticeKind};
use crate::op_dialogs::PlanConfirmDialog;
use crate::script_dialog::{
    BUSY, ContinueDialog, RunScriptDialog, ScriptClaim, ScriptDialog, prepare,
};
use crate::tests::{TestApp, World};

/// Two folders under `work/`: `l` with `n` files, `r` empty.
fn folders(world: &World, n: usize) {
    for i in 0..n {
        world.write(&format!("work/l/f{i:02}.txt"), "left\n");
    }
    std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
}

/// Handles the mailbox and polls until `done` holds (60 s at most).
fn run_until(world: &World, app: &mut TestApp, mut done: impl FnMut(&mut TestApp) -> bool) {
    let began = Instant::now();
    loop {
        for msg in world.mailbox.drain() {
            app.handle(msg);
        }
        app.handle(UiMsg::Tick);
        if done(app) {
            return;
        }
        assert!(
            began.elapsed() < Duration::from_secs(60),
            "the run did not get there"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The status window of the focused window.
fn status(app: &mut TestApp) -> &mut ScriptDialog {
    let w = app.win();
    app.window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<ScriptDialog>())
        .expect("a status window")
}

/// True once the status window's run ended.
fn ended(app: &mut TestApp) -> bool {
    status(app).end().is_some() && !status(app).running()
}

/// Starts `source` (written to `work/s.txt`) as a `RunScript` of the command line would.
fn start(world: &World, app: &mut TestApp, source: &str, args: &[&str]) {
    world.write("work/s.txt", source);
    let claim = ScriptClaim::take(&world.shared).expect("the slot is free");
    let mut start = prepare(
        &world.service,
        OsStr::new("s.txt"),
        args.iter().map(Into::into).collect(),
        None,
        false,
        None,
        None,
    )
    .expect("the script checks");
    start.claim = Some(claim);
    app.handle(UiMsg::RunScript(Box::new(start)));
}

/// The Run Script dialog of the focused window.
fn dialog(app: &mut TestApp) -> &mut RunScriptDialog {
    let w = app.win();
    app.window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<RunScriptDialog>())
        .expect("the dialog")
}

/// The lines of the status window, without their kinds.
fn lines(app: &mut TestApp) -> Vec<String> {
    status(app).lines().map(|(_, l)| l.to_owned()).collect()
}

#[test]
fn a8_19_the_status_window_renders_the_run_and_ends_with_a_notice_and_log_entries() {
    let world = World::new();
    folders(&world, 3);
    let mut app = world.app(None);
    start(
        &world,
        &mut app,
        "option confirm:yes-to-all\nload l r\nselect all\ncopy left->right\nbeep\n",
        &[],
    );
    assert_eq!(status(&mut app).title(), "Script — s.txt");
    assert!(status(&mut app).path().ends_with("s.txt"));
    run_until(&world, &mut app, ended);
    let d = status(&mut app);
    assert_eq!(
        d.command_line().as_deref(),
        Some("Command 5 of 5, line 5: beep")
    );
    assert_eq!(d.end().map(|e| e.exit_code), Some(0));
    assert!(
        d.end_line()
            .is_some_and(|l| l.starts_with("Script completed in ")),
        "{:?}",
        d.end_line()
    );
    assert!(
        d.progress_seen() > 0,
        "the calls' progress came as ScriptEvent::Progress"
    );
    assert_eq!(d.beeps(), 1);
    assert_eq!(d.summary().map(|s| (s.commands, s.ok)), Some((5, 5)));
    let shown = lines(&mut app);
    assert!(shown.iter().any(|l| l == "run L2 load l r"), "{shown:?}");
    assert!(
        shown
            .iter()
            .any(|l| l.starts_with("info L4 copied 3 of 3 items")),
        "{shown:?}"
    );
    assert!(
        shown
            .last()
            .is_some_and(|l| l.starts_with("info - Script completed in "))
    );
    assert!(world.path("work/r/f02.txt").exists(), "the copy ran");
    // The window's buttons: Close after the run, Esc closes.
    assert_eq!(status(&mut app).buttons()[0].role, ButtonRole::Close);
    // The end notice (information) and the log panel: a summary per command, source `script s.txt`.
    let notices = crate::tests_folder_ops::notices(&app);
    assert!(
        notices
            .iter()
            .any(|(kind, text, _)| *kind == NoticeKind::Info && text == "Script s.txt completed"),
        "{notices:?}"
    );
    let summaries: Vec<String> = app
        .log()
        .entries()
        .filter(|e| e.kind == LogKind::Summary && e.source == "script s.txt")
        .map(|e| e.text.clone())
        .collect();
    assert!(
        summaries
            .iter()
            .any(|t| t.starts_with("L4 copied 3 of 3 items")),
        "{summaries:?}"
    );
    let w = app.win();
    let key = Shortcut::parse("Esc").expect("a key");
    app.dialog_key(w, &key);
    assert!(
        app.window(w).is_some_and(|w| w.dialogs.is_empty()),
        "Esc closes it"
    );
    assert!(!world.shared.script_running(), "the slot is free again");
}

#[test]
fn a8_19_a_failed_run_is_an_error_notice_with_show_and_error_entries() {
    let world = World::new();
    folders(&world, 1);
    let mut app = world.app(None);
    start(&world, &mut app, "load l r\nload nowhere r\nbeep\n", &[]);
    run_until(&world, &mut app, ended);
    let d = status(&mut app);
    let id = d.id();
    assert_eq!(d.end().map(|e| e.exit_code), Some(107));
    assert_eq!(d.end_line().as_deref(), Some("Script stopped at line 2"));
    assert!(d.lines().any(|(kind, _)| kind == LineKind::Error));
    let notices = crate::tests_folder_ops::notices(&app);
    assert!(
        notices
            .iter()
            .any(|(kind, text, actions)| *kind == NoticeKind::Error
                && text == "Script s.txt failed: exit 107"
                && actions == &[NoticeAction::ShowScript(id)]),
        "{notices:?}"
    );
    assert!(
        app.log()
            .entries()
            .any(|e| e.kind == LogKind::Error && e.source == "script s.txt"),
        "the error line is a log entry"
    );
    // A failed run keeps the window open (no --closescript, the option is off).
    let w = app.win();
    assert!(
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<ScriptDialog>())
    );
    let effects = app.notice_action(w, "script:s.txt", NoticeAction::ShowScript(id));
    assert!(
        effects.contains(&crate::app::Effect::Raise(w)),
        "{effects:?}"
    );
}

#[test]
fn a8_19_the_log_lines_are_capped_at_display_up_to() {
    let world = World::with_files(&[("options.toml", "[tweaks.log_panel]\ndisplay_up_to = 10\n")]);
    let mut app = world.app(None);
    start(&world, &mut app, &"beep\n".repeat(20), &[]);
    run_until(&world, &mut app, ended);
    let shown = lines(&mut app);
    assert_eq!(shown.len(), 10, "{shown:?}");
    assert!(
        shown[9].starts_with("info - Script completed in "),
        "the newest lines stay"
    );
}

#[test]
fn a8_19_a_question_is_a_confirmation_dialog_whose_answer_reaches_the_run() {
    let world = World::new();
    folders(&world, 2);
    let mut app = world.app(None);
    start(
        &world,
        &mut app,
        "load l r\nselect all\ncopy left->right\n",
        &[],
    );
    let w = app.win();
    run_until(&world, &mut app, |app| {
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<PlanConfirmDialog>())
    });
    let question = app
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<PlanConfirmDialog>())
        .expect("the question");
    assert!(question.for_a_script());
    assert!(
        question.subject().starts_with("L3 copy left->right"),
        "{}",
        question.subject()
    );
    assert!(
        question.do_not_ask_key().is_none(),
        "no option is written for a script"
    );
    app.dialog_button(w, crate::op_dialogs::YES_TO_ALL);
    run_until(&world, &mut app, ended);
    assert_eq!(status(&mut app).end().map(|e| e.exit_code), Some(0));
    assert!(
        world.path("work/r/f01.txt").exists(),
        "the answer reached the run"
    );
    // Cancel declines that command only: it fails, the run goes on (exit 100).
    let world = World::new();
    folders(&world, 2);
    let mut app = world.app(None);
    start(
        &world,
        &mut app,
        "load l r\nselect all\ncopy left->right\nbeep\n",
        &[],
    );
    let w = app.win();
    run_until(&world, &mut app, |app| {
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<PlanConfirmDialog>())
    });
    app.dialog_button(w, ButtonRole::Cancel);
    run_until(&world, &mut app, ended);
    let d = status(&mut app);
    assert_eq!(d.end().map(|e| e.exit_code), Some(100));
    assert_eq!(d.summary().map(|s| (s.failed, s.ok)), Some((1, 3)));
    assert!(!world.path("work/r/f01.txt").exists());
}

#[test]
fn a8_19_stop_cancels_the_run_and_the_rest_is_not_run() {
    let world = World::new();
    folders(&world, 2);
    let mut app = world.app(None);
    start(
        &world,
        &mut app,
        "load l r\nselect all\ncopy left->right\nbeep\n",
        &[],
    );
    let w = app.win();
    run_until(&world, &mut app, |app| {
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<PlanConfirmDialog>())
    });
    // *Stop* (the status window's Cancel role) while the run waits: the question is declined, the job cancelled.
    let service = world.service.clone();
    let step = status(&mut app).on_button(ButtonRole::Cancel, &service);
    assert_eq!(step, crate::dialog::DialogStep::Stay);
    run_until(&world, &mut app, ended);
    let d = status(&mut app);
    assert_eq!(d.end().map(|e| e.exit_code), Some(100));
    assert_eq!(d.end().and_then(|e| e.stopped_at), Some(3));
    assert_eq!(d.end_line().as_deref(), Some("Script stopped at line 3"));
    assert_eq!(d.summary().map(|s| s.not_run), Some(1));
    assert!(!world.path("work/r/f01.txt").exists());
    // The question that was left on the stack goes without effect.
    app.dialog_button(w, ButtonRole::Cancel);
    assert!(
        app.window(w)
            .is_some_and(|w| !w.dialogs.holds::<PlanConfirmDialog>())
    );
}

#[test]
fn a8_19_stop_on_error_asks_yes_or_no() {
    for (yes, beeps, code) in [(true, 2, 100), (false, 1, 100)] {
        let world = World::new();
        let mut app = world.app(None);
        world.write("work/s.txt", "beep\nsnapshot x.hksnap\nbeep\n");
        let claim = ScriptClaim::take(&world.shared).expect("free");
        let mut ready = prepare(
            &world.service,
            OsStr::new("s.txt"),
            Vec::new(),
            None,
            true,
            None,
            None,
        )
        .expect("checks");
        ready.claim = Some(claim);
        app.handle(UiMsg::RunScript(Box::new(ready)));
        let w = app.win();
        run_until(&world, &mut app, |app| {
            app.window(w)
                .is_some_and(|w| w.dialogs.holds::<ContinueDialog>())
        });
        let question = app
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<ContinueDialog>())
            .expect("the question");
        assert!(
            question
                .message()
                .starts_with("The command at line 2 failed: "),
            "{}",
            question.message()
        );
        assert!(question.message().ends_with(". Continue the script?"));
        app.dialog_button(w, if yes { ButtonRole::Yes } else { ButtonRole::No });
        run_until(&world, &mut app, ended);
        let d = status(&mut app);
        assert_eq!(d.beeps(), beeps, "yes = {yes}");
        assert_eq!(d.end().map(|e| e.exit_code), Some(code));
        assert_eq!(d.end().and_then(|e| e.stopped_at), (!yes).then_some(2));
    }
}

#[test]
fn a8_19_close_and_beep_options_and_closescript() {
    // `close_when_finished` closes after exit 0 only; `beep_when_finished` beeps at the end.
    let options = "[tweaks.scripts]\nclose_when_finished = true\nbeep_when_finished = true\n";
    let world = World::with_files(&[("options.toml", options)]);
    let mut app = world.app(None);
    start(&world, &mut app, "beep\n", &[]);
    let w = app.win();
    run_until(&world, &mut app, |app| {
        app.window(w)
            .is_some_and(|w| !w.dialogs.holds::<ScriptDialog>())
    });
    assert!(!world.shared.script_running());
    // A failure keeps it open, with the end's beep.
    start(&world, &mut app, "load nowhere r\n", &[]);
    run_until(&world, &mut app, ended);
    assert_eq!(status(&mut app).beeps(), 1, "beep_when_finished");
    assert!(
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<ScriptDialog>())
    );
    app.dialog_button(w, ButtonRole::Close);
    // `--closescript` closes after any run, whatever the option says.
    let world = World::new();
    let mut app = world.app(None);
    world.write("work/s.txt", "load nowhere r\n");
    let claim = ScriptClaim::take(&world.shared).expect("free");
    let mut ready = prepare(
        &world.service,
        OsStr::new("s.txt"),
        Vec::new(),
        None,
        false,
        None,
        Some(true),
    )
    .expect("checks");
    ready.claim = Some(claim);
    app.handle(UiMsg::RunScript(Box::new(ready)));
    let w = app.win();
    run_until(&world, &mut app, |app| {
        app.window(w)
            .is_some_and(|w| !w.dialogs.holds::<ScriptDialog>())
    });
    // Without the option a run that ended with 0 stays open.
    start(&world, &mut app, "beep\n", &[]);
    run_until(&world, &mut app, ended);
    assert_eq!(status(&mut app).beeps(), 1, "only the script's beep");
    assert!(
        app.window(w)
            .is_some_and(|w| w.dialogs.holds::<ScriptDialog>())
    );
}

#[test]
fn a8_19_a_second_run_script_while_one_runs_is_busy() {
    let world = World::new();
    world.write("work/s.txt", "beep\n");
    world.write("work/bad.txt", "selct all\n");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let request = |path: &str| Request::RunScript {
        path: world.path(path).display().to_string(),
        args: vec![],
        cwd: Some(world.path("work").display().to_string()),
        env: Default::default(),
        wait: false,
        stop_on_error: false,
        confirm: Some("yes-to-all".into()),
        close: None,
    };
    let claim = ScriptClaim::take(&world.shared).expect("free");
    assert_eq!(
        handler.handle_script(request("work/s.txt"), None),
        Response::error(hikasa_session::ipc::IpcErrorCode::Busy, BUSY)
    );
    drop(claim);
    // A script that does not check is `invalid_request` with its first error, and frees the slot.
    let bad = handler.handle_script(request("work/bad.txt"), None);
    assert!(
        matches!(&bad, Response::Error { code, message } if code == "invalid_request" && message.contains("bad.txt:1:1: error:")),
        "{bad:?}"
    );
    assert!(!world.shared.script_running());
    assert_eq!(
        handler.handle_script(request("work/s.txt"), None),
        Response::Ok
    );
    assert!(world.shared.script_running(), "held until the run ends");
    assert!(matches!(
        handler.handle_script(request("work/s.txt"), None),
        Response::Error { code, .. } if code == "busy"
    ));
    // The posted run opens its status window in the window focused last and frees the slot at its end.
    let mut app = world.app(None);
    run_until(&world, &mut app, ended);
    assert!(!world.shared.script_running());
    // A relative cwd is refused.
    let mut relative = request("work/s.txt");
    if let Request::RunScript { cwd, .. } = &mut relative {
        *cwd = Some("work".into());
    }
    assert!(matches!(
        handler.handle_script(relative, None),
        Response::Error { code, .. } if code == "invalid_request"
    ));
}

#[test]
fn a8_19_the_run_script_command_checks_first_and_splits_its_arguments() {
    let world = World::new();
    folders(&world, 1);
    world.write("work/bad.txt", "load l r\nselct all\n");
    world.write("work/s.txt", "load %1 %2\n");
    let mut app = world.app(None);
    let w = app.win();
    assert!(app.enabled(CommandId::RunScript));
    app.execute(CommandId::RunScript);
    assert!(!dialog(&mut app).buttons()[0].enabled, "Run needs a script");
    dialog(&mut app).path = "bad.txt".into();
    app.dialog_button(w, ButtonRole::Ok);
    let errors = dialog(&mut app).errors().to_vec();
    assert!(
        errors.iter().any(|e| e.contains("bad.txt:2:1: error:")),
        "the errors with their positions instead of a run: {errors:?}"
    );
    assert!(!world.shared.script_running(), "nothing ran");
    // An argument field the lexer refuses.
    dialog(&mut app).path = "s.txt".into();
    dialog(&mut app).args = "\"open".into();
    app.dialog_button(w, ButtonRole::Ok);
    assert!(
        dialog(&mut app).errors()[0].starts_with("arguments:1:1: error:"),
        "{:?}",
        dialog(&mut app).errors()
    );
    // Split by the script's word rules: `"l"` is `l`.
    dialog(&mut app).args = "l \"r\"".into();
    app.dialog_button(w, ButtonRole::Ok);
    assert!(
        app.window(w)
            .is_some_and(|w| !w.dialogs.holds::<RunScriptDialog>()),
        "Run closed the dialog"
    );
    run_until(&world, &mut app, ended);
    assert!(lines(&mut app).iter().any(|l| l == "run L1 load l r"));
    app.dialog_button(w, ButtonRole::Close);
    // Esc is Cancel.
    app.execute(CommandId::RunScript);
    let key = Shortcut::parse("Esc").expect("a key");
    app.dialog_key(w, &key);
    assert!(app.window(w).is_some_and(|w| w.dialogs.is_empty()));
    // While a script holds the slot, Run says so.
    let claim = ScriptClaim::take(&world.shared).expect("free");
    app.execute(CommandId::RunScript);
    dialog(&mut app).path = "s.txt".into();
    dialog(&mut app).args = "l r".into();
    app.dialog_button(w, ButtonRole::Ok);
    assert_eq!(dialog(&mut app).errors(), [format!("A{}.", &BUSY[1..])]);
    drop(claim);
}

#[test]
fn a8_19_a_status_window_that_goes_stops_its_run() {
    let world = World::new();
    folders(&world, 2);
    let wake: Wake = Arc::new(|| {});
    world.write(
        "work/s.txt",
        "load l r\nselect all\ncopy left->right\nbeep\n",
    );
    let ready = prepare(
        &world.service,
        OsStr::new("s.txt"),
        Vec::new(),
        None,
        false,
        None,
        None,
    )
    .expect("checks");
    let mut d = ScriptDialog::start(
        ready,
        100,
        false,
        &wake,
        Arc::new(hikasa_session::clipboard::NoClipboard),
    );
    // The run waits on its question; the window goes (the program quits).
    let service = world.service.clone();
    let began = Instant::now();
    while !d
        .take_requests()
        .iter()
        .any(|r| matches!(r, crate::dialog::DialogRequest::Push(_)))
    {
        d.poll(&service);
        assert!(began.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(d.on_close(), "the close button is Stop while it runs");
    while d.running() {
        d.poll(&service);
        assert!(began.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(d.end().map(|e| e.exit_code), Some(100));
    assert!(!d.on_close(), "after the run it closes");
}

/// The status window that owns a question gets its answer (review F1): a failed run keeps its status window in the
/// first window; the next run asks in the second; *Yes to all* there reaches that run, which copies and ends.
#[test]
fn a8_19_a_question_answered_in_a_second_window_reaches_its_own_run() {
    let world = World::new();
    folders(&world, 2);
    let mut app = world.app(None);
    let first = app.win();
    app.execute(CommandId::WindowNew);
    let second = app.windows().last().expect("two windows").id;
    assert_ne!(first, second);
    app.focus(first);
    start(&world, &mut app, "load nowhere r\n", &[]);
    run_until(&world, &mut app, ended);
    let first_status = status(&mut app).id();
    assert!(
        app.window(first)
            .is_some_and(|w| w.dialogs.holds::<ScriptDialog>()),
        "the failed run's status window stays"
    );
    app.focus(second);
    assert_eq!(app.win(), second);
    start(
        &world,
        &mut app,
        "load l r\nselect all\ncopy left->right\n",
        &[],
    );
    run_until(&world, &mut app, |app| {
        app.window(second)
            .is_some_and(|w| w.dialogs.holds::<PlanConfirmDialog>())
    });
    app.dialog_button(second, crate::op_dialogs::YES_TO_ALL);
    run_until(&world, &mut app, ended);
    assert_eq!(status(&mut app).end().map(|e| e.exit_code), Some(0));
    assert!(world.path("work/r/f01.txt").exists(), "the copy ran");
    // The stop-on-error question of a run in the second window, answered there.
    app.dialog_button(second, ButtonRole::Close);
    world.write("work/s.txt", "snapshot x.hksnap\nbeep\n");
    let claim = ScriptClaim::take(&world.shared).expect("free");
    let mut ready = prepare(
        &world.service,
        OsStr::new("s.txt"),
        Vec::new(),
        None,
        true,
        None,
        None,
    )
    .expect("checks");
    ready.claim = Some(claim);
    app.handle(UiMsg::RunScript(Box::new(ready)));
    run_until(&world, &mut app, |app| {
        app.window(second)
            .is_some_and(|w| w.dialogs.holds::<ContinueDialog>())
    });
    app.dialog_button(second, ButtonRole::Yes);
    run_until(&world, &mut app, ended);
    assert_eq!(status(&mut app).beeps(), 1, "Yes reached the run");
    // *Show* raises the window of the status window the notice names.
    let second_status = status(&mut app).id();
    assert_ne!(first_status, second_status);
    let effects = app.notice_action(
        second,
        "script:s.txt",
        NoticeAction::ShowScript(second_status),
    );
    assert!(
        effects.contains(&crate::app::Effect::Raise(second)),
        "{effects:?}"
    );
    // The first run's notice is in the first window.
    app.focus(second);
    let effects = app.notice_action(
        first,
        "script:s.txt",
        NoticeAction::ShowScript(first_status),
    );
    assert!(
        effects.contains(&crate::app::Effect::Raise(first)),
        "{effects:?}"
    );
}
