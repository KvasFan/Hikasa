//! M2 W6d after its review (§1.6.25 *W6d after review*): Disable editing and `--nobackups` survive every route a view's
//! session takes; a question names its row and is not asked once the selection moved; a click during a refresh wins; an
//! operation whose view ends is cancelled, waited for and logged; a pair has no preset side; an Expand waits for a task;
//! a failed archive open is not remembered. Then the tests the review's mutations showed missing. Files live in the
//! World's private temporary folder; no test calls the recycle bin (deletes here are permanent, or cancelled).

use std::sync::Arc;

use hikasa_core::{ContentKind, ContentResult, JobKind, Pair};
use hikasa_folder::NodeId;
use hikasa_folder::ops::{Conflicts, FolderOp, QuestionKind};
use hikasa_session::api::{FolderOpRequest, OpRequest, SelectRequest};
use hikasa_session::ipc::{CloseReason, ViewId};
use hikasa_session::log::LogKind;
use hikasa_session::sessions::{Session, SessionName, SessionRef, SessionType};
use hikasa_vfs::ops::{ItemStamp, OverwritePolicy};

use crate::app::{AppCore, OpenHow, OpenTarget, ViewModel};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::folder_view::{WritePolicy, child_outcome};
use crate::notify::NoticeKind;
use crate::op_dialogs::{NO_TO_ALL, PlanConfirmDialog, SideChoice, YES_TO_ALL};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{
    confirm_dialog, content_of, delete_dialog, dialogs, gate, key, logged, node_of, notices, open,
    open_folder_with_rules, open_with, poll_until, press, put, read, select, selected_path,
    set_gate, shown, top_title,
};

// ---- helpers ------------------------------------------------------------------------------------------------------

fn write_policy(app: &mut AppCore, view: ViewId) -> WritePolicy {
    match app.tab(view).and_then(|t| t.model()) {
        Some(ViewModel::Folder(m)) => m.write_policy(),
        Some(ViewModel::Text(m)) => m.write_policy(),
        _ => panic!("no view {view:?}"),
    }
}

fn protected() -> WritePolicy {
    let mut policy = WritePolicy::default();
    policy.read_only.right = true;
    policy.no_backups = true;
    policy
}

/// A named session of `work/l` against `work/r` with the right side read-only.
fn guarded_session(world: &World) -> SessionRef {
    let mut s = Session::builtin(SessionType::FolderCompare);
    s.specs.left = world.path("work/l").display().to_string();
    s.specs.right = world.path("work/r").display().to_string();
    s.specs.read_only_right = true;
    let reference = SessionRef::parse("guarded").expect("ref");
    world
        .service
        .session_save(&reference, &s, false)
        .expect("save");
    reference
}

fn open_guarded(world: &World, app: &mut TestApp, reference: &SessionRef) -> ViewId {
    world.settle(app);
    let target = OpenTarget::Window(app.win());
    app.0.open_named(reference.clone(), OpenHow::Target(target));
    world.settle(app);
    let view = app.active_tab().expect("tab").id;
    let w = app.win();
    app.0.execute(w, CommandId::FavCompareAll);
    view
}

/// Session ▸ Settings… ▸ OK, nothing changed.
fn settings_ok(world: &World, app: &mut TestApp) {
    let w = app.win();
    app.0.execute(w, CommandId::SessionEdit);
    world.settle(app);
    assert_eq!(top_title(app).as_deref(), Some("Session Settings"));
    press(world, app, ButtonRole::Ok);
    world.settle(app);
}

/// Starts a Copy to Right of `path` and holds its execute job at the gate: the plan passes, the question is answered
/// Yes, and the copy waits (§6.6 "no timers").
fn copy_held_at_execute(
    world: &World,
    app: &mut TestApp,
    view: ViewId,
    path: &str,
) -> crate::folder_view::Gate {
    let g = gate(false);
    folder_model(app, view).gate = Some(Arc::clone(&g));
    select(app, view, path);
    app.execute(CommandId::CopyRight);
    set_gate(&g, true);
    world.settle(app);
    assert_eq!(dialogs(app), 1, "the question");
    set_gate(&g, false);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Yes);
    poll_until(world, app, |a| {
        a.jobs(w).iter().any(|j| j.title == "Copy to Right")
    });
    g
}

fn copy_notices(app: &TestApp) -> Vec<(NoticeKind, String)> {
    notices(app)
        .into_iter()
        .filter(|(_, t, _)| t.starts_with("Copy to Right"))
        .map(|(k, t, _)| (k, t))
        .collect()
}

fn copied_entries(app: &TestApp) -> Vec<String> {
    logged(app, LogKind::Summary)
        .into_iter()
        .filter(|t| t.starts_with("copied "))
        .collect()
}

/// The stamps `Open` would give a row (FC-029(c)).
fn ticket_of(app: &mut AppCore, view: ViewId, path: &str) -> crate::folder_view::ChildTicket {
    let id = node_of(app, view, path).expect("row");
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let read = model.read();
    let node = read.node(id).expect("node");
    let stamp = |e: Option<&hikasa_folder::EntryView<'_>>| {
        let e = e.expect("both sides");
        ItemStamp {
            kind: e.kind,
            size: Some(e.size),
            mtime: e.mtime,
        }
    };
    crate::folder_view::ChildTicket {
        path: read.path(id),
        stamps: Pair::new(stamp(node.left.as_ref()), stamp(node.right.as_ref())),
    }
}

// ---- finding 1: Disable editing and --nobackups survive -----------------------------------------------------------

/// Session ▸ Settings… ▸ OK on a `--ro2 --nobackups` view keeps both: Copy to Right stays off and the right file is
/// never written (it was reset to all-writable before the fix).
#[test]
fn settings_ok_keeps_disable_editing_and_nobackups() {
    let world = World::new();
    put(&world, "l", "pair.txt", "new left\n", 2_000);
    put(&world, "r", "pair.txt", "protected right\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| {
        o.read_only.right = true;
        o.no_backups = true;
    });
    assert_eq!(write_policy(&mut app, view), protected());
    settings_ok(&world, &mut app);
    assert_eq!(write_policy(&mut app, view), protected());
    let w = app.win();
    app.0.execute(w, CommandId::FavCompareAll);
    select(&mut app, view, "pair.txt");
    assert!(!app.enabled(CommandId::CopyRight));
    assert!(app.enabled(CommandId::CopyLeft));
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(
        read(&world, "r", "pair.txt").as_deref(),
        Some("protected right\n")
    );
}

/// A named session with `read_only_right`: protected when it opens (M26), after Settings OK, and its file keeps
/// `read_only_right` through Save.
#[test]
fn a_session_read_only_side_survives_open_settings_and_save() {
    let world = World::new();
    put(&world, "l", "pair.txt", "new left\n", 2_000);
    put(&world, "r", "pair.txt", "protected right\n", 1_000);
    let reference = guarded_session(&world);
    let mut app = world.app(None);
    let view = open_guarded(&world, &mut app, &reference);
    assert!(write_policy(&mut app, view).read_only.right, "at open");
    assert!(!write_policy(&mut app, view).read_only.left);
    select(&mut app, view, "pair.txt");
    assert!(!app.enabled(CommandId::CopyRight));
    settings_ok(&world, &mut app);
    assert!(
        write_policy(&mut app, view).read_only.right,
        "after Settings OK"
    );
    let w = app.win();
    app.0.execute(w, CommandId::SessionSave);
    world.settle(&mut app);
    let file = world.service.session_load(&reference).expect("load").value;
    assert!(file.specs.read_only_right, "Save keeps read_only_right");
    assert!(!file.specs.read_only_left);
    assert_eq!(
        read(&world, "r", "pair.txt").as_deref(),
        Some("protected right\n")
    );
}

/// A plain Session ▸ Save of a protected named session, nothing else done, keeps `read_only_right` in the file.
#[test]
fn a_plain_save_keeps_read_only_in_the_file() {
    let world = World::new();
    put(&world, "l", "pair.txt", "l\n", 2_000);
    put(&world, "r", "pair.txt", "r\n", 1_000);
    let reference = guarded_session(&world);
    let mut app = world.app(None);
    open_guarded(&world, &mut app, &reference);
    let w = app.win();
    app.0.execute(w, CommandId::SessionSave);
    world.settle(&mut app);
    let file = world.service.session_load(&reference).expect("load").value;
    assert!(file.specs.read_only_right);
}

/// Save As of a `--ro2` view writes `read_only_right`, and the session opens protected again.
#[test]
fn save_as_of_a_protected_view_writes_its_read_only_side() {
    let world = World::new();
    put(&world, "l", "pair.txt", "l\n", 2_000);
    put(&world, "r", "pair.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.right = true);
    let w = app.win();
    app.0.execute(w, CommandId::SessionSaveAs);
    world.settle(&mut app);
    app.window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::session_dialog::SaveSessionDialog>()
        })
        .expect("Save As")
        .name = "kept".into();
    press(&world, &mut app, ButtonRole::Ok);
    world.settle(&mut app);
    let reference = SessionRef::parse("kept").expect("ref");
    let file = world.service.session_load(&reference).expect("saved").value;
    assert!(file.specs.read_only_right && !file.specs.read_only_left);
    assert!(write_policy(&mut app, view).read_only.right);
    let again = open_guarded(&world, &mut app, &reference);
    assert_ne!(again, view);
    assert!(write_policy(&mut app, again).read_only.right);
}

/// TabDuplicate copies the write policy (M27), of a folder view and of a text view.
#[test]
fn tab_duplicate_copies_the_write_policy() {
    let world = World::new();
    put(&world, "l", "pair.txt", "l\n", 2_000);
    put(&world, "r", "pair.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| {
        o.read_only.right = true;
        o.no_backups = true;
    });
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let copy = app.active_tab().expect("tab").id;
    assert_ne!(copy, view);
    assert_eq!(write_policy(&mut app, copy), protected());
    // A text view keeps it too (its session does).
    let text = world.open(
        &mut app,
        &world.path("work/l/pair.txt"),
        &world.path("work/r/pair.txt"),
    );
    world.settle(&mut app);
    if let Some(ViewModel::Text(m)) = app.tab_mut(text).and_then(|t| t.model_mut()) {
        m.set_write_policy(protected());
    } else {
        panic!("a text view");
    }
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let copy = app.active_tab().expect("tab").id;
    assert_ne!(copy, text);
    assert_eq!(write_policy(&mut app, copy), protected());
}

/// A workspace keeps an unnamed view's read-only side (in its inline session) and `--nobackups` (in the tab's `view`
/// table, which no session has), and restores both.
#[test]
fn a_workspace_keeps_the_write_policy() {
    let world = World::new();
    put(&world, "l", "pair.txt", "l\n", 2_000);
    put(&world, "r", "pair.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| {
        o.read_only.right = true;
        o.no_backups = true;
    });
    let w = app.win();
    app.0.execute(w, CommandId::WorkspaceSaveAs);
    world.settle(&mut app);
    app.window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::workspace::SaveWorkspaceDialog>()
        })
        .expect("Save Workspace As")
        .name = "w".into();
    press(&world, &mut app, ButtonRole::Ok);
    world.settle(&mut app);
    let name = SessionName::parse("w").expect("name");
    let saved = world
        .service
        .workspace_load(&name)
        .expect("saved")
        .workspace;
    let tab = saved.windows[0]
        .tabs
        .iter()
        .find(|t| !t.inline.is_empty())
        .expect("the inline tab");
    assert_eq!(
        tab.view.get("no_backups").and_then(|v| v.as_bool()),
        Some(true)
    );
    // Load it: the views close, the tab is restored and started.
    app.0.load_workspace(name);
    world.settle(&mut app);
    for index in 0..app.tabs().len() {
        app.activate(index);
        world.settle(&mut app);
    }
    let restored: Vec<ViewId> = app
        .tabs()
        .iter()
        .filter(|t| matches!(t.model(), Some(ViewModel::Folder(_))))
        .map(|t| t.id)
        .collect();
    assert_eq!(restored.len(), 1, "one folder tab");
    assert_ne!(restored[0], view);
    assert_eq!(write_policy(&mut app, restored[0]), protected());
}

// ---- finding 2: the question names its row; a moved selection cancels -------------------------------------------

/// The selection moves while the plan is made: no question, nothing written, a warning notice says why.
#[test]
fn a_selection_moved_while_planning_cancels_the_operation() {
    let world = World::new();
    put(&world, "l", "a.txt", "aaaa\n", 1_000);
    put(&world, "l", "b.txt", "bbbb\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    select(&mut app, view, "b.txt");
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(
        dialogs(&app),
        0,
        "no question about a row no longer selected"
    );
    assert_eq!(read(&world, "r", "a.txt"), None);
    assert_eq!(read(&world, "r", "b.txt"), None);
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert_eq!(
        text,
        "Copy to Right: the selection changed while the operation was prepared; nothing was written."
    );
    // The view is free again, and the new row can be copied.
    assert!(app.enabled(CommandId::CopyRight));
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(
        confirm_dialog(&mut app).subject(),
        "b.txt: from the left to the right"
    );
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(read(&world, "r", "b.txt").as_deref(), Some("bbbb\n"));
}

/// The selection moves while the Delete dialog is open: OK plans nothing, no question, a warning.
#[test]
fn a_selection_moved_under_the_delete_dialog_cancels_it() {
    let world = World::new();
    put(&world, "l", "a.txt", "aaaa\n", 1_000);
    put(&world, "l", "b.txt", "bbbb\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::PermanentDelete);
    assert_eq!(delete_dialog(&mut app).path(), "a.txt");
    select(&mut app, view, "b.txt");
    press(&world, &mut app, ButtonRole::Ok);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(text.contains("the selection changed"), "{text}");
    assert!(read(&world, "l", "a.txt").is_some() && read(&world, "l", "b.txt").is_some());
}

/// The question names the row and its sides: a copy each way, a delete of one side and of both.
#[test]
fn the_question_names_the_row_and_its_sides() {
    let world = World::new();
    put(&world, "l", "d/pair.txt", "left\n", 2_000);
    put(&world, "r", "d/pair.txt", "right!\n", 2_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    for (command, subject) in [
        (
            CommandId::CopyRight,
            "d/pair.txt: from the left to the right",
        ),
        (
            CommandId::CopyLeft,
            "d/pair.txt: from the right to the left",
        ),
    ] {
        select(&mut app, view, "d/pair.txt");
        app.execute(command);
        world.settle(&mut app);
        assert_eq!(confirm_dialog(&mut app).subject(), subject);
        press(&world, &mut app, ButtonRole::Cancel);
    }
    for (choice, subject) in [
        (SideChoice::Left, "d/pair.txt: on the left"),
        (SideChoice::Right, "d/pair.txt: on the right"),
        (SideChoice::Both, "d/pair.txt: on both sides"),
    ] {
        app.execute(CommandId::PermanentDelete);
        assert_eq!(delete_dialog(&mut app).path(), "d/pair.txt");
        delete_dialog(&mut app).set_choice(choice);
        press(&world, &mut app, ButtonRole::Ok);
        assert_eq!(confirm_dialog(&mut app).subject(), subject);
        press(&world, &mut app, ButtonRole::Cancel);
    }
    assert!(read(&world, "l", "d/pair.txt").is_some() && read(&world, "r", "d/pair.txt").is_some());
}

// ---- finding 3: a click during a refresh wins ---------------------------------------------------------------------

/// A row selected and a folder closed while a refresh runs stay so: the refresh puts back only what the user left as
/// it was. Copy to Right then copies the row the user selected.
#[test]
fn a_click_during_a_refresh_wins() {
    let world = World::new();
    put(&world, "l", "a.txt", "aaaa\n", 1_000);
    put(&world, "l", "b.txt", "bbbb\n", 1_000);
    put(&world, "l", "d/x.txt", "x\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    select(&mut app, view, "a.txt");
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    let service = world.service.clone();
    folder_model(&mut app, view).refresh_for_test(&service, vec![NodeId::ROOT]);
    // While it runs: another row, and d closed.
    select(&mut app, view, "b.txt");
    select(&mut app, view, "d");
    app.execute(CommandId::Collapse);
    select(&mut app, view, "b.txt");
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(selected_path(&mut app, view).as_deref(), Some("b.txt"));
    let rows = shown(&mut app, view);
    assert!(
        !rows.iter().any(|(p, _)| p == "d/x.txt"),
        "d stays closed: {rows:?}"
    );
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(read(&world, "r", "b.txt").as_deref(), Some("bbbb\n"));
    assert_eq!(read(&world, "r", "a.txt"), None);
}

/// A full re-run: a row the user selects while it compares again stays selected (`after_run`), and while it compares
/// no row is operable (M24: the commands are off and the view refuses by itself).
#[test]
fn a_click_during_a_rerun_wins_and_nothing_operates_while_comparing() {
    let world = World::new();
    put(&world, "l", "a.txt", "aaaa\n", 1_000);
    put(&world, "l", "b.txt", "bbbb\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    let g = gate(false);
    folder_model(&mut app, view).scan_gate = Some(Arc::clone(&g));
    app.execute(CommandId::FullRefresh);
    // The new comparison has scanned and waits before it reports: the view is still comparing.
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        m.scanning() && m.prepared().is_some_and(|p| p.model().read().len() > 2)
    });
    folder_model(&mut app, view).rebuild();
    select(&mut app, view, "b.txt");
    assert!(folder_model(&mut app, view).scanning());
    for id in [
        CommandId::CopyRight,
        CommandId::Delete,
        CommandId::PermanentDelete,
    ] {
        assert!(!app.enabled(id), "{id:?} while comparing");
    }
    let service = world.service.clone();
    let started = folder_model(&mut app, view).start_operation(
        &service,
        view,
        CommandId::Delete,
        "Delete",
        false,
    );
    assert!(started.is_none());
    let w = app.win();
    assert!(
        app.jobs(w).iter().all(|j| !j.title.starts_with("plan")),
        "{:?}",
        app.jobs(w)
    );
    set_gate(&g, true);
    world.settle(&mut app);
    assert!(!folder_model(&mut app, view).scanning());
    assert_eq!(selected_path(&mut app, view).as_deref(), Some("b.txt"));
    assert_eq!(dialogs(&app), 0);
}

// ---- finding 4: an operation whose view ends is cancelled, waited for and logged ----------------------------------

/// Closing the tab while its copy runs: the copy is cancelled, ends, and its outcome reaches the log with one notice
/// naming the closed tab (deterministic: the job waits at a gate until after the tab closed).
#[test]
fn closing_the_tab_while_a_copy_runs_logs_its_outcome() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = copy_held_at_execute(&world, &mut app, view, "a.txt");
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.close_tab(index, CloseReason::User);
    assert!(app.tab(view).is_none());
    assert!(copied_entries(&app).is_empty(), "not ended yet");
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        a.log().entries().any(|e| e.text.starts_with("copied "))
    });
    let entries: Vec<(String, String)> = app
        .log()
        .entries()
        .filter(|e| e.text.starts_with("copied "))
        .map(|e| (e.source.clone(), e.text.clone()))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, "l ⇄ r");
    let copy = copy_notices(&app);
    assert_eq!(copy.len(), 1, "one notice: {copy:?}");
    assert_eq!(copy[0].0, NoticeKind::Warning);
    assert!(
        copy[0]
            .1
            .starts_with("Copy to Right in l ⇄ r (closed): stopped; copied 0 of 1"),
        "{}",
        copy[0].1
    );
    assert_eq!(
        read(&world, "r", "a.txt"),
        None,
        "cancelled before its item"
    );
}

/// A copy that ended but whose outcome the window has not taken yet when the tab closes: the outcome is reported all the
/// same (the view hands over what it had not), as a whole copy.
#[test]
fn an_outcome_not_taken_yet_when_the_tab_closes_is_reported() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = copy_held_at_execute(&world, &mut app, view, "a.txt");
    for _ in world.mailbox.drain() {}
    set_gate(&g, true);
    // The job's body ended (its wake-up is in the mailbox) and its result is published; nobody polled the window.
    for _ in 0..60_000 {
        if !world.mailbox.is_empty() && !folder_model(&mut app, view).settling() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!world.mailbox.is_empty(), "the copy ended");
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("left\n"));
    assert!(copied_entries(&app).is_empty(), "not taken yet");
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.close_tab(index, CloseReason::User);
    let entries = copied_entries(&app);
    assert_eq!(entries.len(), 1, "{entries:?}");
    let copy = copy_notices(&app);
    assert_eq!(copy.len(), 1, "{copy:?}");
    assert!(
        copy[0]
            .1
            .starts_with("Copy to Right in l ⇄ r (closed): copied 1 of 1"),
        "{}",
        copy[0].1
    );
}

/// Session ▸ Settings… ▸ OK while a copy runs replaces the view's model: the copy is cancelled and still logged.
#[test]
fn settings_ok_while_a_copy_runs_logs_its_outcome() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = copy_held_at_execute(&world, &mut app, view, "a.txt");
    let w = app.win();
    app.0.execute(w, CommandId::SessionEdit);
    poll_until(&world, &mut app, |a| {
        a.window(a.focused())
            .and_then(|w| w.dialogs.top())
            .is_some_and(|d| d.title() == "Session Settings")
    });
    app.0.dialog_button(w, ButtonRole::Ok);
    // The view takes its new model: the copy is no longer the view's job.
    poll_until(&world, &mut app, |a| {
        !a.jobs(w).iter().any(|j| j.title == "Copy to Right")
    });
    assert!(copied_entries(&app).is_empty());
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        a.log().entries().any(|e| e.text.starts_with("copied "))
    });
    world.settle(&mut app);
    let copy = copy_notices(&app);
    assert_eq!(copy.len(), 1, "{copy:?}");
    assert!(
        copy[0]
            .1
            .starts_with("Copy to Right in l ⇄ r (settings applied): stopped;"),
        "{}",
        copy[0].1
    );
    assert!(app.tab(view).is_some(), "the view stays");
}

/// Quit while a copy runs waits for it (bounded) before it returns, and logs it; nothing is left behind in the target
/// folder. The gate opens only once the shutdown waits, so a shutdown that did not wait would find nothing logged.
#[test]
fn quit_waits_for_a_running_copy_and_logs_it() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = copy_held_at_execute(&world, &mut app, view, "a.txt");
    let opener = Arc::clone(&g);
    app.0.before_wait = Some(Box::new(move || set_gate(&opener, true)));
    app.0.shutdown();
    assert!(app.0.before_wait.is_none(), "the shutdown waited");
    let entries = copied_entries(&app);
    assert_eq!(entries.len(), 1, "{entries:?}");
    let names: Vec<String> = std::fs::read_dir(world.path("work/r"))
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.is_empty(), "nothing left behind: {names:?}");
}

// ---- finding 5: a pair has no preset side -------------------------------------------------------------------------

/// A pair whose right side is read-only: Shift+Del, Enter, Enter deletes nothing: no side is preset, OK stays off.
#[test]
fn a_pair_with_a_read_only_side_presets_nothing() {
    let world = World::new();
    put(&world, "l", "pair.txt", "left\n", 2_000);
    put(&world, "r", "pair.txt", "right!\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.right = true);
    select(&mut app, view, "pair.txt");
    app.execute(CommandId::PermanentDelete);
    assert_eq!(delete_dialog(&mut app).choice(), None);
    key(&world, &mut app, "Enter");
    key(&world, &mut app, "Enter");
    assert_eq!(top_title(&app).as_deref(), Some("Delete Permanently"));
    key(&world, &mut app, "Esc");
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "l", "pair.txt").as_deref(), Some("left\n"));
    assert_eq!(read(&world, "r", "pair.txt").as_deref(), Some("right!\n"));
}

// ---- finding 6: an Expand waits for a task ------------------------------------------------------------------------

/// Expand on a folder that was not scanned while a refresh runs: it waits, and once the refresh ended the folder is
/// refreshed and opened.
#[test]
fn an_expand_during_a_task_runs_when_it_ends() {
    let world = World::new();
    put(&world, "l", "a/b/x.txt", "x\n", 1_000);
    put(&world, "r", "a/other.txt", "o\n", 1_000);
    let mut app = world.app(None);
    let view = open_folder_with_rules(&world, &mut app, |p| {
        p.auto_scan_subfolders = Some(false);
    });
    app.execute(CommandId::ExpandAll);
    app.execute(CommandId::CollapseAll);
    select(&mut app, view, "a");
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    let service = world.service.clone();
    folder_model(&mut app, view).refresh_for_test(&service, vec![NodeId::ROOT]);
    select(&mut app, view, "a/b");
    app.execute(CommandId::Expand);
    set_gate(&g, true);
    world.settle(&mut app);
    let rows = shown(&mut app, view);
    assert!(rows.iter().any(|(p, _)| p == "a/b/x.txt"), "{rows:?}");
}

// ---- finding 7: a failed archive open is not remembered -----------------------------------------------------------

/// Open on an archive that cannot be read: the view's session does not keep it, a warning says why.
#[test]
fn a_failed_archive_open_is_not_remembered() {
    let world = World::new();
    std::fs::create_dir_all(world.path("work/l")).expect("mkdir");
    std::fs::write(world.path("work/l/z.zip"), b"PK\x03\x04 not a zip at all").expect("zip");
    std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "z.zip");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(folder_model(&mut app, view).open_archives(), None);
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(text.starts_with("Open z.zip: "), "{text}");
    // A good one is kept.
    let zip = crate::tests_shell::stored_zip(&[("m.txt", b"member\n")]);
    std::fs::write(world.path("work/l/y.zip"), &zip).expect("zip");
    let view = open(&world, &mut app);
    select(&mut app, view, "y.zip");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(
        folder_model(&mut app, view).open_archives(),
        Some(&["y.zip".to_owned()][..])
    );
}

// ---- the mutations the review showed surviving --------------------------------------------------------------------

/// M13: Delete Permanently of one side of a pair, chosen in the dialog, deletes that side only.
#[test]
fn a_delete_of_one_side_of_a_pair_deletes_only_that_side() {
    let world = World::new();
    put(&world, "l", "pair.txt", "left\n", 2_000);
    put(&world, "r", "pair.txt", "right!\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    for (choice, gone, kept) in [(SideChoice::Left, "l", "r"), (SideChoice::Right, "r", "l")] {
        put(&world, "l", "pair.txt", "left\n", 2_000);
        put(&world, "r", "pair.txt", "right!\n", 1_000);
        app.execute(CommandId::FullRefresh);
        world.settle(&mut app);
        select(&mut app, view, "pair.txt");
        app.execute(CommandId::PermanentDelete);
        delete_dialog(&mut app).set_choice(choice);
        press(&world, &mut app, ButtonRole::Ok);
        assert_eq!(confirm_dialog(&mut app).counts(), ["remove: 1 items"]);
        press(&world, &mut app, ButtonRole::Yes);
        assert_eq!(read(&world, gone, "pair.txt"), None, "{choice:?}");
        assert!(
            read(&world, kept, "pair.txt").is_some(),
            "{choice:?}: the other side stays"
        );
    }
}

/// M07: the plan request carries the read-only sides, and the Service's refusal (D-184) shows: a side that became
/// read-only while the Delete dialog was open is refused by the Service, with a warning, and nothing is deleted.
#[test]
fn the_service_refuses_a_read_only_side_and_the_window_says_so() {
    let world = World::new();
    put(&world, "r", "right-only.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "right-only.txt");
    app.execute(CommandId::PermanentDelete);
    assert_eq!(delete_dialog(&mut app).choice(), Some(SideChoice::Right));
    let mut policy = WritePolicy::default();
    policy.read_only.right = true;
    folder_model(&mut app, view).set_write_policy(policy);
    press(&world, &mut app, ButtonRole::Ok);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0, "no question: nothing was planned");
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert_eq!(text, "Delete Permanently: the right side is read-only");
    assert!(read(&world, "r", "right-only.txt").is_some());
}

/// M08: the view refuses a copy into a read-only side by itself (not only through the predicates): no plan job starts.
#[test]
fn a_copy_into_a_read_only_side_is_refused_by_the_view() {
    let world = World::new();
    put(&world, "l", "pair.txt", "left\n", 2_000);
    put(&world, "r", "pair.txt", "right!\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.right = true);
    select(&mut app, view, "pair.txt");
    let service = world.service.clone();
    let started = folder_model(&mut app, view).start_operation(
        &service,
        view,
        CommandId::CopyRight,
        "Copy to Right",
        false,
    );
    assert!(started.is_none());
    let w = app.win();
    assert!(app.jobs(w).is_empty(), "{:?}", app.jobs(w));
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert!(notices(&app).is_empty(), "{:?}", notices(&app));
    assert_eq!(read(&world, "r", "pair.txt").as_deref(), Some("right!\n"));
}

/// M10: "Do not ask again" ticked with Yes, then Cancel on a later question: the operation is cancelled and no key is
/// written.
#[test]
fn do_not_ask_again_then_cancel_writes_no_key() {
    let world = World::new();
    put(&world, "l", "a.txt", "old left\n", 1_000);
    put(&world, "r", "a.txt", "newer right\n", 2_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    let d = confirm_dialog(&mut app);
    assert_eq!(d.kind(), Some(QuestionKind::Operation));
    d.ticked = true;
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(
        confirm_dialog(&mut app).kind(),
        Some(QuestionKind::OverwriteNewer)
    );
    press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert!(app.options().file_operations.confirm_copy);
    let options = std::fs::read_to_string(world.path("config/options.toml")).unwrap_or_default();
    assert!(!options.contains("confirm_copy"), "{options}");
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("newer right\n"));
}

/// M18: Yes to the operation, then the dialog closes with an item's question open (the window's close button): a
/// Cancel; not even the items that had no question run.
#[test]
fn a_dialog_closed_mid_questions_runs_no_item() {
    let world = World::new();
    put(&world, "l", "d/x.txt", "new\n", 1_000);
    put(&world, "l", "d/y.txt", "old left\n", 1_000);
    put(&world, "r", "d/y.txt", "newer right\n", 2_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(
        confirm_dialog(&mut app).kind(),
        Some(QuestionKind::OverwriteNewer)
    );
    let w = app.win();
    app.0.close_window(w);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "d/x.txt"), None, "no item ran");
    assert_eq!(
        read(&world, "r", "d/y.txt").as_deref(),
        Some("newer right\n")
    );
    assert!(
        logged(&app, LogKind::Summary)
            .iter()
            .any(|t| t == "Copy to Right: cancelled, nothing was written."),
        "{:?}",
        logged(&app, LogKind::Summary)
    );
}

/// M20 (A6.34 "through renumbered"): a folder the user opens while a refresh runs is open after it, though the refresh
/// gave its row a new id: only `renumbered` can move it (it was not open when the refresh started, so no path names it).
#[test]
fn renumbered_moves_a_folder_opened_during_a_refresh() {
    let world = World::new();
    put(&world, "l", "d/x.txt", "x\n", 1_000);
    put(&world, "r", "d/x.txt", "x\n", 1_000);
    put(&world, "l", "d/e/y.txt", "y\n", 1_000);
    put(&world, "r", "d/e/y.txt", "y\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    let d = node_of(&mut app, view, "d").expect("d");
    let before = node_of(&mut app, view, "d/e").expect("d/e");
    // A new file in d: its refresh replaces d's records (a row whose folder changed is renumbered).
    put(&world, "l", "d/new.txt", "n\n", 1_000);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    let service = world.service.clone();
    folder_model(&mut app, view).refresh_for_test(&service, vec![d]);
    select(&mut app, view, "d/e");
    app.execute(CommandId::Expand);
    set_gate(&g, true);
    world.settle(&mut app);
    let after = node_of(&mut app, view, "d/e").expect("d/e");
    assert_ne!(before, after, "the refresh renumbered d/e");
    let rows = shown(&mut app, view);
    assert!(
        rows.iter().any(|(p, _)| p == "d/e/y.txt"),
        "d/e stays open: {rows:?}"
    );
    assert_eq!(selected_path(&mut app, view).as_deref(), Some("d/e"));
}

/// M02: the model changes while the plan is made: no question is asked, nothing runs, a warning says so.
#[test]
fn a_model_changed_while_planning_asks_nothing() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    put(&world, "l", "b.txt", "other\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let b = node_of(&mut app, view, "b.txt").expect("b");
    world.service.folder_set_ignored(&model, &[b], true);
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "a.txt"), None);
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(text.contains("the comparison changed"), "{text}");
}

/// M23: FullRefresh while an operation runs does nothing: the operation ends, is reported and refreshes its rows in
/// the same model.
#[test]
fn full_refresh_waits_for_an_operation() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let g = copy_held_at_execute(&world, &mut app, view, "a.txt");
    app.execute(CommandId::FullRefresh);
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("left\n"));
    let copy = copy_notices(&app);
    assert_eq!(copy.len(), 1, "{copy:?}");
    assert!(
        copy[0].1.starts_with("Copy to Right: copied 1 of 1"),
        "{}",
        copy[0].1
    );
    let now = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    assert!(Arc::ptr_eq(&model, &now), "no re-run");
}

/// M35: the window passes `require_side_selection_in_delete`: a one-sided row has no preset then.
#[test]
fn the_window_passes_require_side_selection_in_delete() {
    let world = World::with_files(&[(
        "options.toml",
        "[file_operations]\nrequire_side_selection_in_delete = true\n",
    )]);
    put(&world, "l", "left-only.txt", "l\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "left-only.txt");
    app.execute(CommandId::Delete);
    let d = delete_dialog(&mut app);
    assert_eq!(d.choice(), None);
    assert!(
        d.buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && !b.enabled)
    );
    press(&world, &mut app, ButtonRole::Cancel);
}

/// M36: a child's result that arrives while a question is open waits for the operation: the plan still holds (Yes
/// runs it), and the result reaches its row afterwards.
#[test]
fn a_child_result_waits_for_an_open_question() {
    let world = World::new();
    put(&world, "l", "c.txt", "one line\n", 1_000);
    put(&world, "r", "c.txt", "one line\r\n", 1_000);
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let ticket = ticket_of(&mut app, view, "c.txt");
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1);
    let child = ViewId(9_999);
    let m = folder_model(&mut app, view);
    m.adopt_child(child, ticket);
    m.child_result(child, child_outcome(ContentResult::RulesSame));
    app.handle(crate::app::UiMsg::Tick);
    assert_eq!(
        content_of(&mut app, view, "c.txt"),
        None,
        "not under a question"
    );
    press(&world, &mut app, ButtonRole::Yes);
    world.settle(&mut app);
    assert_eq!(
        read(&world, "r", "a.txt").as_deref(),
        Some("left\n"),
        "the plan held"
    );
    assert_eq!(
        content_of(&mut app, view, "c.txt"),
        Some(ContentKind::RulesSame)
    );
}

// ---- A6.33 "each question": the item questions through the dialog -------------------------------------------------

/// A read-only target: its question, its key, Enter is No; No keeps it, Yes replaces it.
#[test]
fn the_read_only_question() {
    let world = World::new();
    put(&world, "l", "pair.txt", "new left\n", 2_000);
    put(&world, "r", "pair.txt", "old right\n", 1_000);
    let target = world.path("work/r/pair.txt");
    let mut perms = std::fs::metadata(&target).expect("meta").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&target, perms).expect("read-only");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    for (reply, after) in [
        (ButtonRole::No, "old right\n"),
        (ButtonRole::Yes, "new left\n"),
    ] {
        select(&mut app, view, "pair.txt");
        app.execute(CommandId::CopyRight);
        world.settle(&mut app);
        press(&world, &mut app, ButtonRole::Yes);
        let d = confirm_dialog(&mut app);
        assert_eq!(d.kind(), Some(QuestionKind::ReadOnly));
        assert_eq!(d.question(), Some("Replace or delete read-only pair.txt?"));
        assert_eq!(
            d.do_not_ask_key(),
            Some("file_operations.confirm_read_only")
        );
        assert!(
            d.buttons()
                .iter()
                .any(|b| b.role == ButtonRole::No && b.default)
        );
        press(&world, &mut app, reply);
        assert_eq!(dialogs(&app), 0);
        assert_eq!(
            read(&world, "r", "pair.txt").as_deref(),
            Some(after),
            "{reply:?}"
        );
    }
}

/// A backup that would replace a file this plan did not make: its question (the `confirm_overwrite_newer` key); No
/// keeps the target and the old backup.
#[test]
fn the_overwrite_backup_question() {
    let world = World::with_files(&[(
        "options.toml",
        "[backups]\nback_up_before_copy = true\n[file_operations]\nconfirm_copy = false\n",
    )]);
    put(&world, "l", "a.txt", "new left\n", 2_000);
    put(&world, "r", "a.txt", "old\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0, "the first backup is new");
    let listing = |world: &World| {
        let mut names: Vec<(String, String)> = std::fs::read_dir(world.path("work/r"))
            .expect("list")
            .map(|e| {
                let e = e.expect("entry");
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(e.path()).unwrap_or_default(),
                )
            })
            .collect();
        names.sort();
        names
    };
    let before = listing(&world);
    assert_eq!(before.len(), 2, "{before:?}");
    put(&world, "l", "a.txt", "newest left\n", 3_000);
    app.execute(CommandId::FullRefresh);
    world.settle(&mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    let d = confirm_dialog(&mut app);
    assert_eq!(d.kind(), Some(QuestionKind::OverwriteBackup));
    assert!(
        d.question()
            .is_some_and(|q| q.starts_with("The backup of a.txt would replace"))
    );
    assert_eq!(
        d.do_not_ask_key(),
        Some("file_operations.confirm_overwrite_newer")
    );
    press(&world, &mut app, ButtonRole::No);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(
        listing(&world),
        before,
        "No keeps the target and the old backup"
    );
}

/// The questions the window's two commands cannot raise on this system, through the dialog all the same: a move that
/// replaces (its plan from the Service) and a system file (a copy's plan with the conflict a Windows system file has).
/// Each: its text, its key, Enter is No; No to All declines the class; Yes to All accepts it.
#[test]
fn the_replace_during_move_and_system_questions() {
    let world = World::new();
    for name in ["a.txt", "b.txt"] {
        put(&world, "l", name, "left\n", 2_000);
        put(&world, "r", name, "right!\n", 1_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let mut selection = hikasa_folder::select::Selection::default();
    for name in ["a.txt", "b.txt"] {
        let node = node_of(&mut app, view, name).expect("row");
        selection.insert(node, hikasa_folder::select::SideSet::LEFT);
    }
    let plan_of = |op: FolderOp| {
        let request = FolderOpRequest {
            op: OpRequest::Op(op),
            select: SelectRequest {
                selection: Some(selection.clone()),
                expand: hikasa_session::api::ExpandWords::All,
                ..SelectRequest::default()
            },
            read_only: Default::default(),
            overrides: hikasa_session::api::OpOverrides::default(),
        };
        let model = Arc::clone(&model);
        world
            .service
            .spawn(JobKind::FolderOp, "plan".into(), move |svc, job| {
                svc.folder_op_plan(&model, &request, job)
            })
            .wait()
            .expect("plan")
    };
    let moved = plan_of(FolderOp::Move {
        to: hikasa_core::Side::Right,
        overwrite: OverwritePolicy::Always,
    });
    let mut copied = plan_of(FolderOp::Copy {
        to: hikasa_core::Side::Right,
        overwrite: OverwritePolicy::Always,
    });
    for meta in &mut copied.meta {
        meta.conflicts |= Conflicts::SYSTEM;
    }
    let service = world.service.clone();
    for (plan, kind, key, text) in [
        (
            moved,
            QuestionKind::ReplaceDuringMove,
            "file_operations.confirm_replace_during_move",
            "Replace a.txt with the moved file?",
        ),
        (
            copied,
            QuestionKind::System,
            "file_operations.confirm_system",
            "Replace or delete system file a.txt?",
        ),
    ] {
        for (reply, runs) in [(NO_TO_ALL, 0), (YES_TO_ALL, 2)] {
            let confirmation = world.service.folder_confirmation(&plan, None);
            let mut d = PlanConfirmDialog::new(
                ViewId(1),
                "t",
                "a, b: from the left to the right",
                &plan,
                confirmation,
            );
            assert_eq!(d.kind(), Some(QuestionKind::Operation));
            d.on_button(ButtonRole::Yes, &service);
            assert_eq!(d.kind(), Some(kind));
            assert_eq!(d.question(), Some(text));
            assert_eq!(d.do_not_ask_key(), Some(key));
            assert!(
                d.buttons()
                    .iter()
                    .any(|b| b.role == ButtonRole::No && b.default)
            );
            assert_eq!(d.remaining(), 2);
            d.on_button(reply, &service);
            assert_eq!(d.question(), None, "the class is answered");
            let finished = d
                .take_confirmation()
                .expect("answered")
                .finish(plan.clone())
                .expect("not declined");
            let running = finished
                .plan
                .items
                .iter()
                .filter(|i| i.skip.is_none() && i.error.is_none())
                .count();
            assert!(running >= runs, "{kind:?} {reply:?}: {running}");
            if runs == 0 {
                assert_eq!(running, 0, "{kind:?}: No to All declines every item");
            }
        }
    }
    // Nothing was executed.
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("right!\n"));
    assert!(read(&world, "l", "a.txt").is_some());
}
