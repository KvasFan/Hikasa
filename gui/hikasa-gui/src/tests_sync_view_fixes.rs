//! M2 W9b after its review (§1.9.14, §1.9.15, §1.9.25 W9b notes 20 and after): a sync whose view ends, the overrides a
//! plan cannot use, the plan's state, the waited outcome's rows.

use std::sync::Arc;

use hikasa_core::ExitCode;
use hikasa_folder::sync::{SyncAction, SyncOptions, SyncPreset};
use hikasa_session::api::ViewOutcome;
use hikasa_session::ipc::{CloseReason, ViewId};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::folder_view::Gate;
use crate::notify::NoticeKind;
use crate::op_dialogs::{PlanConfirmDialog, YES_TO_ALL};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{gate, notices, poll_until, set_gate};
use crate::tests_sync_view::{
    cli_twin, confirm_all, find, fixture, open_sync, open_sync_with, ticks, tree, trees,
};

fn mirror_right() -> SyncOptions {
    SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    }
}

/// Run Sync answered Yes to All, its execute job held at a gate (no settle: the window would wait for it).
pub(crate) fn run_held(world: &World, app: &mut TestApp, view: ViewId) -> Gate {
    let g = gate(false);
    folder_model(app, view).gate = Some(Arc::clone(&g));
    app.execute(CommandId::Run);
    ticks(world, app);
    assert!(find::<PlanConfirmDialog>(app).is_some(), "the question");
    let w = app.win();
    for _ in 0..6 {
        if find::<PlanConfirmDialog>(app).is_some() {
            app.0.dialog_button(w, YES_TO_ALL);
        }
        for msg in world.mailbox.drain() {
            app.0.handle(msg);
        }
        app.0.handle(crate::app::UiMsg::Tick);
    }
    assert!(find::<PlanConfirmDialog>(app).is_none(), "answered");
    assert!(
        folder_model(app, view).busy(),
        "the sync is held at the gate"
    );
    g
}

fn outcome(world: &World, app: &mut TestApp, view: ViewId) -> ViewOutcome {
    let service = world.service.clone();
    folder_model(app, view).outcome(&service)
}

// ---- A9.17: the waited outcome's rows (review F4, F8, R24) ------------------------------------------------------

/// A Run whose job fails as a whole and a Run that was stopped both make the waited exit code 100; a Run that then
/// succeeds makes it 0 again.
#[test]
fn a9_17_a_failed_or_stopped_run_exits_100() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    // The job fails (its error, not an outcome).
    folder_model(&mut app, view).fail_sync = true;
    app.execute(CommandId::Run);
    ticks(&world, &mut app);
    confirm_all(&world, &mut app);
    folder_model(&mut app, view).fail_sync = false;
    assert_eq!(
        outcome(&world, &mut app, view),
        ViewOutcome::Sync {
            last_run: Some(ExitCode::UnknownError),
            complete: true
        },
        "a failed Run"
    );
    // A Run stopped before its first item.
    ticks(&world, &mut app);
    let g = run_held(&world, &mut app, view);
    app.execute(CommandId::Stop);
    set_gate(&g, true);
    world.settle(&mut app);
    folder_model(&mut app, view).gate = None;
    assert_eq!(
        outcome(&world, &mut app, view),
        ViewOutcome::Sync {
            last_run: Some(ExitCode::UnknownError),
            complete: true
        },
        "a stopped Run"
    );
    // A Run that writes everything.
    ticks(&world, &mut app);
    app.execute(CommandId::Run);
    ticks(&world, &mut app);
    confirm_all(&world, &mut app);
    assert_eq!(
        outcome(&world, &mut app, view),
        ViewOutcome::Sync {
            last_run: Some(ExitCode::Success),
            complete: true
        },
        "a Run that wrote everything"
    );
}

/// A Folder Sync view whose bases cannot be loaded closes as `load_failed` (D-23), not as a sync without a Run.
#[test]
fn a9_17_a_sync_view_that_cannot_load_is_load_failed() {
    use hikasa_session::api::{FolderCompareRequest, ViewRequest};
    use hikasa_session::ipc::{OpenOptions, ViewKind};
    let world = World::new();
    std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
    let mut app = world.app(None);
    let (left, right) = (world.path("work/missing"), world.path("work/r"));
    let request = ViewRequest::Sync {
        request: FolderCompareRequest {
            left: left.clone().into_os_string(),
            right: right.clone().into_os_string(),
            ..FolderCompareRequest::default()
        },
        options: mirror_right(),
        allow_empty_source: false,
        display: hikasa_session::api::resolve_folder_display(None, None, false, false)
            .expect("display"),
    };
    let target = app.open_target();
    let view = app.open_local(
        ViewKind::FolderSync,
        request,
        &OpenOptions::default(),
        vec![
            left.to_string_lossy().into_owned(),
            right.to_string_lossy().into_owned(),
        ],
        target,
    );
    ticks(&world, &mut app);
    let got = outcome(&world, &mut app, view);
    assert_eq!(got, ViewOutcome::LoadFailed);
    let (result, code) = got.close(hikasa_session::ipc::CloseReason::User, false);
    assert_eq!(code, ExitCode::UnknownError);
    assert_eq!(result.reason, "load_failed", "{result:?}");
}

// ---- F1: a sync whose view ends is cancelled, waited for and logged (W6d's finding 4 for a sync) ---------------------

fn sync_notices(app: &TestApp) -> Vec<(NoticeKind, String)> {
    notices(app)
        .into_iter()
        .filter(|(_, t, _)| t.starts_with("Run Sync"))
        .map(|(k, t, _)| (k, t))
        .collect()
}

fn names(world: &World, side: &str) -> Vec<String> {
    let mut names: Vec<String> = tree(world, side).into_keys().collect();
    names.sort();
    names
}

/// Closing the tab while its sync runs: the sync is cancelled, ends, and its outcome reaches the log with one notice
/// naming the closed tab (the job waits at a gate until after the tab closed); nothing was written.
#[test]
fn closing_the_tab_while_a_sync_runs_logs_its_outcome() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    let before = trees(&world);
    let g = run_held(&world, &mut app, view);
    let entries = app.0.log().entries().count();
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.close_tab(index, CloseReason::User);
    assert!(app.tab(view).is_none());
    assert!(sync_notices(&app).is_empty(), "not ended yet");
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        a.window(a.focused()).is_some_and(|w| {
            w.notices
                .all()
                .iter()
                .any(|n| n.text.starts_with("Run Sync"))
        })
    });
    let notes = sync_notices(&app);
    assert_eq!(notes.len(), 1, "one notice: {notes:?}");
    assert_eq!(notes[0].0, NoticeKind::Warning);
    assert!(
        notes[0]
            .1
            .starts_with("Run Sync in l ⇄ r (closed): stopped;"),
        "{}",
        notes[0].1
    );
    let logged: Vec<String> = app
        .0
        .log()
        .entries()
        .skip(entries)
        .filter(|e| e.source == "l ⇄ r")
        .map(|e| e.text.clone())
        .collect();
    assert!(!logged.is_empty(), "the sync's entries are logged");
    assert_eq!(trees(&world), before, "cancelled before its first item");
}

/// A sync that ended but whose outcome the window has not taken yet when the tab closes is reported all the same.
#[test]
fn a_sync_outcome_not_taken_yet_when_the_tab_closes_is_reported() {
    let (world, twin) = (World::new(), World::new());
    fixture(&world);
    fixture(&twin);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    let g = run_held(&world, &mut app, view);
    for _ in world.mailbox.drain() {}
    set_gate(&g, true);
    for _ in 0..60_000 {
        if !world.mailbox.is_empty() && !folder_model(&mut app, view).settling() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!world.mailbox.is_empty(), "the sync ended");
    assert!(sync_notices(&app).is_empty(), "not taken yet");
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.close_tab(index, CloseReason::User);
    let notes = sync_notices(&app);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].1.starts_with("Run Sync in l ⇄ r (closed): ") && !notes[0].1.contains("stopped"),
        "{}",
        notes[0].1
    );
    assert_eq!(cli_twin(&twin, mirror_right(), None), ExitCode::Success);
    assert_eq!(trees(&world), trees(&twin), "the whole sync ran");
}

/// Session ▸ Settings… ▸ OK while a sync runs replaces the view's model: the sync is cancelled, logged, and its exit
/// code becomes the view's last Run.
#[test]
fn settings_ok_while_a_sync_runs_logs_its_outcome() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    let g = run_held(&world, &mut app, view);
    let w = app.win();
    app.0.execute(w, CommandId::SessionEdit);
    poll_until(&world, &mut app, |a| {
        a.window(a.focused())
            .and_then(|w| w.dialogs.top())
            .is_some_and(|d| d.title() == "Session Settings")
    });
    app.0.dialog_button(w, ButtonRole::Ok);
    poll_until(&world, &mut app, |a| {
        !a.jobs(w).iter().any(|j| j.title == "Run Sync")
    });
    assert!(sync_notices(&app).is_empty());
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        a.window(a.focused()).is_some_and(|w| {
            w.notices
                .all()
                .iter()
                .any(|n| n.text.starts_with("Run Sync"))
        })
    });
    world.settle(&mut app);
    let notes = sync_notices(&app);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0]
            .1
            .starts_with("Run Sync in l ⇄ r (settings applied): stopped;"),
        "{}",
        notes[0].1
    );
    assert!(app.tab(view).is_some(), "the view stays");
    assert_eq!(
        folder_model(&mut app, view).sync().and_then(|s| s.last_run),
        Some(ExitCode::UnknownError),
        "the stopped sync is the view's last Run"
    );
}

/// Quit while a sync runs waits for it (bounded) and logs it; its current item ends first: nothing is left behind (the
/// gate opens only once the shutdown waits).
#[test]
fn quit_waits_for_a_running_sync_and_logs_it() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    let right = names(&world, "r");
    let g = run_held(&world, &mut app, view);
    let entries = app.0.log().entries().count();
    let opener = Arc::clone(&g);
    app.0.before_wait = Some(Box::new(move || set_gate(&opener, true)));
    app.0.shutdown();
    assert!(app.0.before_wait.is_none(), "the shutdown waited");
    let logged: Vec<String> = app
        .0
        .log()
        .entries()
        .skip(entries)
        .filter(|e| e.source == "l ⇄ r")
        .map(|e| e.text.clone())
        .collect();
    assert!(
        logged
            .iter()
            .any(|t| t.starts_with("synchronized 0 of 5 items") && t.contains("5 not run")),
        "the sync's entries: {logged:?}"
    );
    assert_eq!(names(&world, "r"), right, "nothing left behind");
}

/// A waited Folder Sync view closed while its sync runs answers its waiter when the sync ended, with the sync's own
/// exit code (stopped: 100), not the code of the Run before it or of no Run (0).
#[test]
fn a_waited_sync_view_closed_while_its_sync_runs_exits_with_the_sync_s_code() {
    use hikasa_session::ipc::{
        Endpoint, IpcClient, IpcServer, OpenOptions, Request, Response, ViewKind,
    };
    let world = World::new();
    fixture(&world);
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = crate::ipc::GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = world.app(Some(handle));
    let timeout = std::time::Duration::from_secs(10);
    let specs = vec![
        world.path("work/l").display().to_string(),
        world.path("work/r").display().to_string(),
    ];
    let wait_endpoint = endpoint.clone();
    let waiting = std::thread::spawn(move || {
        let mut client = IpcClient::connect(&wait_endpoint, "test", timeout).expect("connect");
        let (id, response) = client
            .request(Request::OpenView {
                kind: ViewKind::FolderSync,
                specs,
                options: OpenOptions::default(),
                wait: true,
            })
            .expect("open");
        let Response::ViewOpened { view } = response else {
            panic!("{response:?}");
        };
        (view, client.wait_view_closed(id).expect("closed"))
    });
    let began = std::time::Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    world.settle(&mut app);
    ticks(&world, &mut app);
    let view = app.active_tab().expect("tab").id;
    assert!(world.shared.is_waited(view));
    assert!(
        folder_model(&mut app, view)
            .sync()
            .is_some_and(|s| s.plan_ready()),
        "the default preset acts on the fixture"
    );
    let g = run_held(&world, &mut app, view);
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.close_tab(index, CloseReason::User);
    assert!(!world.shared.contains(view), "the view left the list");
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        a.window(a.focused()).is_some_and(|w| {
            w.notices
                .all()
                .iter()
                .any(|n| n.text.starts_with("Run Sync"))
        })
    });
    let (opened, (closed, result, code)) = waiting.join().expect("waiter thread");
    assert_eq!((opened, closed), (view, view));
    assert_eq!(
        code,
        ExitCode::UnknownError.code(),
        "the stopped sync's code"
    );
    assert_eq!(result.close_reason(), Some(CloseReason::User));
}

// ---- F2: only overrides a row cannot take are removed, once the comparison is complete -----------------------------

fn overrides(app: &mut TestApp, view: ViewId) -> Vec<(String, SyncAction)> {
    folder_model(app, view)
        .sync()
        .expect("sync")
        .options
        .overrides
        .iter()
        .map(|(p, a)| (p.clone(), *a))
        .collect()
}

fn dirty(app: &TestApp, view: ViewId) -> bool {
    app.0.view_session(view).is_some_and(|s| s.dirty)
}

/// An override whose row is absent from this comparison (a file not there yet) is kept, the session stays clean, its
/// warning is one notice and the header's list; an edit that plans again names it no more (the reviewer's probe).
#[test]
fn an_override_of_an_absent_row_is_kept_with_a_warning() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let mut options = mirror_right();
    options
        .overrides
        .insert("later/keep.txt".into(), SyncAction::Leave);
    let view = open_sync(&world, &mut app, options);
    ticks(&world, &mut app);
    assert_eq!(
        overrides(&mut app, view),
        [("later/keep.txt".to_owned(), SyncAction::Leave)]
    );
    assert!(!dirty(&app, view), "nothing changed");
    let warned = |app: &TestApp| {
        notices(app)
            .iter()
            .filter(|(k, t, _)| *k == NoticeKind::Warning && t.contains("later/keep.txt"))
            .count()
    };
    assert_eq!(warned(&app), 1);
    assert_eq!(
        folder_model(&mut app, view)
            .sync()
            .expect("sync")
            .unused_overrides(),
        ["later/keep.txt"]
    );
    // Another plan (a preset change) keeps it and names it no more.
    let service = world.service.clone();
    folder_model(&mut app, view).set_sync_preset(&service, SyncPreset::UpdateRight);
    ticks(&world, &mut app);
    assert_eq!(warned(&app), 1, "one notice per path");
    assert!(
        overrides(&mut app, view)
            .iter()
            .any(|(p, _)| p == "later/keep.txt")
    );
    // The file comes: the override is used and leaves the header's list.
    crate::tests_folder_ops::put(&world, "l", "later/keep.txt", "keep\n", 1_000);
    app.execute(CommandId::FullRefresh);
    ticks(&world, &mut app);
    let m = folder_model(&mut app, view);
    assert!(m.sync().expect("sync").unused_overrides().is_empty());
    let node = crate::tests_folder_ops::node_of(&mut app, view, "later/keep.txt").expect("row");
    let action = folder_model(&mut app, view)
        .sync_action(node)
        .expect("planned");
    assert_eq!(
        (action.action, action.overridden),
        (SyncAction::Leave, true)
    );
}

/// An override a row cannot take is kept while the comparison is not whole (here a size filter hides a side of a row),
/// and removed with its notice once a whole comparison planned it. (A Folder Sync view scans every folder; the
/// orphan folders a Folder Compare view leaves unscanned reach a sync only through Synchronize ▸, which removes nothing.)
#[test]
fn a_dropped_override_waits_for_a_whole_comparison() {
    use crate::tests_folder_ops::put;
    let world = World::new();
    put(&world, "l", "tiny.txt", "ab\n", 1_000);
    put(&world, "r", "tiny.txt", "a longer text\n", 1_000);
    put(&world, "l", "big-left.txt", "0123456789\n", 1_000);
    let mut app = world.app(None);
    let mut options = SyncOptions {
        preset: SyncPreset::UpdateRight,
        ..SyncOptions::default()
    };
    // Left only: no right side to copy from.
    options
        .overrides
        .insert("big-left.txt".into(), SyncAction::CopyLeft);
    let patch = hikasa_folder::FolderRulesPatch {
        size_filter: Some(hikasa_folder::Change::Set(hikasa_folder::SizeFilter {
            cmp: hikasa_folder::Cmp::Greater,
            bytes: 5,
        })),
        ..Default::default()
    };
    let view = open_sync_with(&world, &mut app, options, patch);
    ticks(&world, &mut app);
    let plan = folder_model(&mut app, view)
        .sync()
        .and_then(|s| s.plan().cloned())
        .expect("a plan");
    assert!(
        plan.rows
            .iter()
            .any(|r| r.reason == Some(hikasa_folder::sync::LeaveReason::Filtered)),
        "a side is hidden"
    );
    assert_eq!(plan.dropped_overrides, ["big-left.txt"]);
    assert_eq!(
        overrides(&mut app, view),
        [("big-left.txt".to_owned(), SyncAction::CopyLeft)],
        "kept while a filter hides a side"
    );
    assert!(!dirty(&app, view));
    // The hidden side grows past the filter: the comparison is whole, the override goes, with its notice.
    put(&world, "l", "tiny.txt", "now long enough\n", 1_000);
    app.execute(CommandId::FullRefresh);
    ticks(&world, &mut app);
    assert!(overrides(&mut app, view).is_empty(), "removed");
    assert!(dirty(&app, view));
    assert!(
        notices(&app)
            .iter()
            .any(|(k, t, _)| *k == NoticeKind::Warning && t.starts_with("override big-left.txt=")),
        "{:?}",
        notices(&app)
    );
}

// ---- F7, F12: the plan Run Sync runs and the actions shown are those of the current options and comparison ----------

/// After a change of the preset, Run Sync waits for the new plan: the old one is gone at once (no Run of the previous
/// options in the same frame or from a script), and the Run that follows writes what the new preset writes.
#[test]
fn run_sync_waits_for_the_plan_of_the_new_options() {
    let (world, twin) = (World::new(), World::new());
    fixture(&world);
    fixture(&twin);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    assert!(app.enabled(CommandId::Run));
    let service = world.service.clone();
    assert!(folder_model(&mut app, view).set_sync_preset(&service, SyncPreset::UpdateRight));
    assert!(!app.enabled(CommandId::Run), "the old plan is gone");
    assert!(
        folder_model(&mut app, view)
            .sync()
            .expect("sync")
            .plan()
            .is_none()
    );
    assert!(
        !folder_model(&mut app, view).run_sync(&service, view, "Run Sync"),
        "nothing to run before the new plan"
    );
    assert!(find::<PlanConfirmDialog>(&mut app).is_none());
    ticks(&world, &mut app);
    assert!(app.enabled(CommandId::Run));
    app.execute(CommandId::Run);
    ticks(&world, &mut app);
    confirm_all(&world, &mut app);
    let options = SyncOptions {
        preset: SyncPreset::UpdateRight,
        ..SyncOptions::default()
    };
    assert_eq!(cli_twin(&twin, options, None), ExitCode::Success);
    assert_eq!(trees(&world), trees(&twin), "Update Right, not Mirror");
}

/// While the comparison runs again, and once a refresh changed it, the rows show no action of the old plan and Info no
/// counts of it; the plan of the new comparison brings them back.
#[test]
fn the_old_plan_is_not_shown_on_a_changed_comparison() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, mirror_right());
    let a = crate::tests_folder_ops::node_of(&mut app, view, "a.txt").expect("row");
    assert!(folder_model(&mut app, view).sync_action(a).is_some());
    assert!(folder_model(&mut app, view).sync_counts().is_some());
    // A refresh of the rows changes the model's generation: the plan no longer describes it.
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    let root = hikasa_folder::NodeId::ROOT;
    world
        .service
        .spawn(
            hikasa_core::JobKind::FolderRefresh,
            "refresh".into(),
            move |svc, job| svc.folder_refresh(&model, &[root], job, &hikasa_folder::NullObserver),
        )
        .wait()
        .expect("refresh");
    assert!(
        folder_model(&mut app, view).sync_action(a).is_none(),
        "stale"
    );
    assert!(
        folder_model(&mut app, view).sync_counts().is_none(),
        "stale"
    );
    ticks(&world, &mut app);
    let a = crate::tests_folder_ops::node_of(&mut app, view, "a.txt").expect("row");
    assert!(
        folder_model(&mut app, view).sync_action(a).is_some(),
        "planned again"
    );
    // A full re-run held at its scan gate: nothing of the old plan while it compares.
    let g = gate(false);
    folder_model(&mut app, view).scan_gate = Some(Arc::clone(&g));
    app.execute(CommandId::FullRefresh);
    poll_until(&world, &mut app, |a| folder_model(a, view).scanning());
    assert!(folder_model(&mut app, view).sync_action(a).is_none());
    assert!(folder_model(&mut app, view).sync_counts().is_none());
    assert!(!app.enabled(CommandId::Run));
    set_gate(&g, true);
    ticks(&world, &mut app);
    let a = crate::tests_folder_ops::node_of(&mut app, view, "a.txt").expect("row");
    assert!(folder_model(&mut app, view).sync_action(a).is_some());
    assert!(folder_model(&mut app, view).sync_counts().is_some());
}

// ---- F11: the Synchronize preview after the comparison changed ------------------------------------------------------

/// Run after the comparison changed under the Synchronize preview runs nothing: the dialog says so and offers *Plan
/// again*, which opens the preview once more on the comparison as it stands.
#[test]
fn the_synchronize_preview_offers_plan_again_after_a_change() {
    use crate::dialog::DialogModel;
    use crate::folder_view::sync::SyncDialog;
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = crate::tests_folder_ops::open(&world, &mut app);
    let before = trees(&world);
    app.execute(CommandId::MirrorRight);
    ticks(&world, &mut app);
    assert!(find::<SyncDialog>(&mut app).is_some_and(|d| d.list().is_some()));
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    let root = hikasa_folder::NodeId::ROOT;
    world
        .service
        .spawn(
            hikasa_core::JobKind::FolderRefresh,
            "refresh".into(),
            move |svc, job| svc.folder_refresh(&model, &[root], job, &hikasa_folder::NullObserver),
        )
        .wait()
        .expect("refresh");
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Ok);
    let d = find::<SyncDialog>(&mut app).expect("still open");
    assert!(
        d.message().is_some_and(|m| m.contains("Plan again")),
        "{:?}",
        d.message()
    );
    assert!(d.buttons().iter().any(|b| b.label == "Plan again"));
    assert!(!d.buttons().iter().any(|b| b.role == ButtonRole::Ok));
    crate::tests_folder_ops::press(&world, &mut app, crate::op_dialogs::PLAN_AGAIN);
    ticks(&world, &mut app);
    let d = find::<SyncDialog>(&mut app).expect("the command ran again");
    assert!(d.message().is_none() && d.list().is_some());
    assert_eq!(trees(&world), before, "nothing was written");
}
