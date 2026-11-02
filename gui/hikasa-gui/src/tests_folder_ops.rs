//! M2 W6d (§1.6.20, D-189): the folder operations of the window through the catalogue, without a window (§1.4.16):
//! the four commands and their predicates, the confirmation and Delete dialogs, the flow from the plan to the log, the
//! notice and the refresh in the view (A6.33, A6.34). Files live in a private temporary folder; the recycle bin is never
//! called (a delete to the bin is planned and cancelled, or runs on an archive member, which has none).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hikasa_core::{ContentKind, ContentResult};
use hikasa_folder::ops::{OpVerb, QuestionKind};
use hikasa_folder::select::SideSet;
use hikasa_folder::{Category, NodeId};
use hikasa_session::api::{GuiOpenRequest, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use hikasa_session::log::LogKind;
use hikasa_session::settings::Shortcut;

use crate::app::{AppCore, ViewModel};
use crate::commands::{CommandId, FocusArea, Scope};
use crate::dialog::{ButtonRole, DialogModel};
use crate::folder_view::{FolderViewModel, Gate};
use crate::notify::{NoticeAction, NoticeKind};
use crate::op_dialogs::{
    DeleteDialog, NO_TO_ALL, PlanConfirmDialog, SideChoice, YES_TO_ALL, option_key,
};
use crate::tests::{TestApp, World, folder_model};

// ---- helpers ------------------------------------------------------------------------------------------------------

/// Sets a file's modification time to `secs` after the epoch (whole seconds, §6.6 "Times").
pub(crate) fn touch(path: &Path, secs: u64) {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open to touch");
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .expect("set time");
}

/// Writes `work/<side>/<rel>` with `text` at time `secs`.
pub(crate) fn put(world: &World, side: &str, rel: &str, text: &str, secs: u64) {
    let path = world.write(&format!("work/{side}/{rel}"), text);
    touch(&path, secs);
}

pub(crate) fn read(world: &World, side: &str, rel: &str) -> Option<String> {
    std::fs::read_to_string(world.path(&format!("work/{side}/{rel}"))).ok()
}

/// Opens `work/l` against `work/r` with `tune` applied to the open options (read-only sides, `--nobackups`).
pub(crate) fn open_with(
    world: &World,
    app: &mut AppCore,
    tune: impl FnOnce(&mut OpenOptions),
) -> ViewId {
    let (left, right) = (world.path("work/l"), world.path("work/r"));
    for side in ["work/l", "work/r"] {
        std::fs::create_dir_all(world.path(side)).expect("mkdir");
    }
    let request = GuiOpenRequest {
        specs: vec![left.into_os_string(), right.into_os_string()],
        ..GuiOpenRequest::default()
    };
    let mut resolved = world.service.resolve_open(&request).expect("resolve");
    tune(&mut resolved.options);
    let view = world
        .service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .expect("view request");
    let target = app.open_target();
    let id = app.open_local(
        resolved.kind,
        view,
        &resolved.options,
        resolved.specs,
        target,
    );
    world.settle(app);
    // Every row shows (the default display is the differences).
    let w = app.focused();
    app.execute(w, CommandId::FavCompareAll);
    id
}

pub(crate) fn open(world: &World, app: &mut AppCore) -> ViewId {
    open_with(world, app, |_| {})
}

/// The node of the row at `path` (a folder when `folder`).
pub(crate) fn node_of(app: &mut AppCore, view: ViewId, path: &str) -> Option<NodeId> {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared()?.model());
    let read = model.read();
    read.preorder()
        .map(|(id, _)| id)
        .find(|&id| id != NodeId::ROOT && read.path(id).as_str() == path)
}

/// Selects the visible row at `path`.
pub(crate) fn select(app: &mut AppCore, view: ViewId, path: &str) {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let index = {
        let read = model.read();
        m.rows()
            .iter()
            .position(|r| read.path(r.node).as_str() == path)
            .unwrap_or_else(|| panic!("no visible row {path}"))
    };
    m.select(index);
}

/// The selected row's path.
pub(crate) fn selected_path(app: &mut AppCore, view: ViewId) -> Option<String> {
    let m = folder_model(app, view);
    let id = m.selected()?;
    let model = Arc::clone(m.prepared()?.model());
    let path = model.read().path(id).as_str().to_owned();
    Some(path)
}

/// What the view shows: (path, category) of every visible row.
pub(crate) fn shown(app: &mut AppCore, view: ViewId) -> Vec<(String, Category)> {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let read = model.read();
    m.rows()
        .iter()
        .map(|r| {
            (
                read.path(r.node).as_str().to_owned(),
                read.category(r.node, false),
            )
        })
        .collect()
}

pub(crate) fn dialogs(app: &TestApp) -> usize {
    app.window(app.win()).map_or(0, |w| w.dialogs.len())
}

pub(crate) fn top_title(app: &TestApp) -> Option<String> {
    app.window(app.win())
        .and_then(|w| w.dialogs.top())
        .map(|d| d.title())
}

pub(crate) fn confirm_dialog(app: &mut TestApp) -> &mut PlanConfirmDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<PlanConfirmDialog>())
        .expect("a confirmation dialog")
}

pub(crate) fn delete_dialog(app: &mut TestApp) -> &mut DeleteDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<DeleteDialog>())
        .expect("a Delete dialog")
}

pub(crate) fn press(world: &World, app: &mut TestApp, role: ButtonRole) {
    let w = app.win();
    app.0.dialog_button(w, role);
    world.settle(app);
}

pub(crate) fn key(world: &World, app: &mut TestApp, text: &str) {
    let w = app.win();
    let shortcut = Shortcut::parse(text).expect("a key");
    app.0.dialog_key(w, &shortcut);
    world.settle(app);
}

/// The log's entries of a kind.
pub(crate) fn logged(app: &TestApp, kind: LogKind) -> Vec<String> {
    app.log()
        .entries()
        .filter(|e| e.kind == kind)
        .map(|e| e.text.clone())
        .collect()
}

/// The window's notices (oldest first): (kind, text, actions).
pub(crate) fn notices(app: &TestApp) -> Vec<(NoticeKind, String, Vec<NoticeAction>)> {
    app.window(app.win())
        .map(|w| {
            w.notices
                .all()
                .iter()
                .map(|n| (n.kind, n.text.clone(), n.actions.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn gate(open: bool) -> Gate {
    Arc::new((std::sync::Mutex::new(open), std::sync::Condvar::new()))
}

pub(crate) fn set_gate(gate: &Gate, open: bool) {
    *gate.0.lock().expect("gate") = open;
    gate.1.notify_all();
}

/// Polls the window until `done` holds (bounded by count, not by a clock: §6.6 "No timers").
pub(crate) fn poll_until(
    world: &World,
    app: &mut AppCore,
    mut done: impl FnMut(&mut AppCore) -> bool,
) {
    for _ in 0..20_000 {
        for msg in world.mailbox.drain() {
            app.handle(msg);
        }
        app.handle(crate::app::UiMsg::Tick);
        if done(app) {
            return;
        }
        std::thread::yield_now();
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the state was not reached");
}

// ---- catalogue, predicates, shortcuts -----------------------------------------------------------------------------

/// A6.33: catalogue parity of the four commands: implemented with their default shortcuts and captions, a row in the
/// folder view, each executed changes the model or opens a dialog; `Del` is in the shortcut table of the tree only.
#[test]
fn a6_33_the_four_commands_are_in_the_catalogue_and_run() {
    let tables = crate::commands::CommandTables::default();
    for (id, caption, shortcut) in [
        (CommandId::CopyRight, "Copy to Right", "Ctrl+R"),
        (CommandId::CopyLeft, "Copy to Left", "Ctrl+L"),
        (CommandId::Delete, "Delete", "Del"),
        (
            CommandId::PermanentDelete,
            "Delete Permanently",
            "Shift+Del",
        ),
    ] {
        assert!(crate::commands::shown(&tables, Scope::Folder, id), "{id:?}");
        assert_eq!(tables.caption(Scope::Folder, id), caption);
        assert_eq!(
            tables.shortcut(Scope::Folder, id),
            Some(&Shortcut::parse(shortcut).expect("shortcut")),
            "{id:?}"
        );
        assert!(!crate::commands::shown(&tables, Scope::Text, id));
    }
    let world = World::new();
    put(&world, "l", "pair.txt", "left\n", 2_000);
    put(&world, "r", "pair.txt", "right!\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "pair.txt");
    let at = |app: &TestApp, key: &str, area| {
        app.dispatch_keys(app.win(), &[Shortcut::parse(key).expect("key")], area)
    };
    assert_eq!(
        at(&app, "Del", FocusArea::ViewMain),
        Some(CommandId::Delete)
    );
    assert_eq!(
        at(&app, "Shift+Del", FocusArea::ViewMain),
        Some(CommandId::PermanentDelete)
    );
    // Del belongs to the focused tree: a path field or the Home tree keeps its own.
    assert_eq!(at(&app, "Del", FocusArea::PathField), None);
    assert_eq!(
        at(&app, "Ctrl+R", FocusArea::Root),
        Some(CommandId::CopyRight)
    );
    for id in [CommandId::Delete, CommandId::PermanentDelete] {
        app.execute(id);
        assert!(dialogs(&app) == 1, "{id:?} opens its dialog");
        press(&world, &mut app, ButtonRole::Cancel);
        assert_eq!(dialogs(&app), 0);
    }
    for id in [CommandId::CopyRight, CommandId::CopyLeft] {
        app.execute(id);
        world.settle(&mut app);
        assert_eq!(dialogs(&app), 1, "{id:?} asks");
        key(&world, &mut app, "Esc");
        assert_eq!(dialogs(&app), 0);
    }
    assert_eq!(read(&world, "r", "pair.txt").as_deref(), Some("right!\n"));
    assert_eq!(read(&world, "l", "pair.txt").as_deref(), Some("left\n"));
}

/// A6.33: the predicates: a row is needed, a copy needs an item on the other side, and nothing runs while the
/// comparison runs or a dialog is open.
#[test]
fn a6_33_predicates_follow_the_row_and_its_sides() {
    let world = World::new();
    put(&world, "l", "left-only.txt", "l\n", 1_000);
    put(&world, "r", "right-only.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let four = [
        CommandId::CopyRight,
        CommandId::CopyLeft,
        CommandId::Delete,
        CommandId::PermanentDelete,
    ];
    for id in four {
        assert!(!app.enabled(id), "{id:?} without a row");
    }
    select(&mut app, view, "left-only.txt");
    assert!(app.enabled(CommandId::CopyRight));
    assert!(!app.enabled(CommandId::CopyLeft), "nothing on the right");
    assert!(app.enabled(CommandId::Delete));
    select(&mut app, view, "right-only.txt");
    assert!(!app.enabled(CommandId::CopyRight));
    assert!(app.enabled(CommandId::CopyLeft));
    app.execute(CommandId::Delete);
    for id in four {
        assert!(!app.enabled(id), "{id:?} while a dialog is open");
    }
    // A second operation is refused while one waits for its dialog: no plan's job starts (a copy hands back no dialog
    // either way, so the job list and the dialog stack say whether it started), and none asks afterwards.
    let (service, w) = (world.service.clone(), app.win());
    let again = folder_model(&mut app, view).start_operation(
        &service,
        view,
        CommandId::CopyLeft,
        "Copy to Left",
        false,
    );
    assert!(again.is_none());
    assert!(app.jobs(w).is_empty(), "{:?}", app.jobs(w));
    world.settle(&mut app);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(1));
    assert_eq!(top_title(&app).as_deref(), Some("Delete"));
    press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0, "no question of the refused copy");
    assert!(app.enabled(CommandId::CopyLeft));
}

/// A6.33: Disable editing: a read-only side turns off what writes it, `Del` included; the Delete dialog greys it; and the
/// Service refuses it too (D-184).
#[test]
fn a6_33_commands_are_off_for_a_read_only_side() {
    let world = World::new();
    put(&world, "l", "pair.txt", "left\n", 2_000);
    put(&world, "r", "pair.txt", "right!\n", 1_000);
    put(&world, "r", "right-only.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.right = true);
    select(&mut app, view, "right-only.txt");
    for id in [
        CommandId::CopyRight,
        CommandId::Delete,
        CommandId::PermanentDelete,
    ] {
        assert!(!app.enabled(id), "{id:?} on a read-only right orphan");
    }
    assert!(
        app.enabled(CommandId::CopyLeft),
        "reading a read-only side is allowed"
    );
    let del = Shortcut::parse("Del").expect("key");
    assert_eq!(
        app.dispatch_keys(app.win(), std::slice::from_ref(&del), FocusArea::ViewMain),
        None
    );
    // The view refuses it even when asked directly.
    let service = world.service.clone();
    for id in [CommandId::CopyRight, CommandId::Delete] {
        let started = folder_model(&mut app, view).start_operation(&service, view, id, "x", false);
        assert!(started.is_none(), "{id:?}");
    }
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    select(&mut app, view, "pair.txt");
    assert!(!app.enabled(CommandId::CopyRight));
    assert!(app.enabled(CommandId::CopyLeft));
    assert!(app.enabled(CommandId::Delete));
    app.execute(CommandId::Delete);
    let d = delete_dialog(&mut app);
    assert!(d.enabled(SideChoice::Left));
    assert!(!d.enabled(SideChoice::Right) && !d.enabled(SideChoice::Both));
    assert_eq!(
        d.choice(),
        None,
        "a pair has no preset, even with one side read-only"
    );
    d.set_choice(SideChoice::Right);
    assert_eq!(d.choice(), None, "a greyed side is not taken");
    d.set_choice(SideChoice::Left);
    assert_eq!(d.choice(), Some(SideChoice::Left));
    press(&world, &mut app, ButtonRole::Cancel);
    // Both sides read-only (`--ro`): nothing writes.
    let both = open_with(&world, &mut app, |o| o.read_only.all = true);
    select(&mut app, both, "pair.txt");
    for id in [
        CommandId::CopyRight,
        CommandId::CopyLeft,
        CommandId::Delete,
        CommandId::PermanentDelete,
    ] {
        assert!(!app.enabled(id), "{id:?} with --ro");
    }
    assert_eq!(
        app.dispatch_keys(app.win(), &[del], FocusArea::ViewMain),
        None
    );
    assert_eq!(read(&world, "r", "pair.txt").as_deref(), Some("right!\n"));
}

// ---- the confirmation dialog --------------------------------------------------------------------------------------

/// A6.33: "Do not ask again" names the key of its question, and the sync's question and the bin's have none.
#[test]
fn a6_33_do_not_ask_again_keys() {
    use QuestionKind as Q;
    assert_eq!(
        option_key(Q::Operation, OpVerb::Copy),
        Some("file_operations.confirm_copy")
    );
    assert_eq!(
        option_key(Q::Operation, OpVerb::Delete),
        Some("file_operations.confirm_delete")
    );
    assert_eq!(
        option_key(Q::Operation, OpVerb::Move),
        Some("file_operations.confirm_move")
    );
    assert_eq!(option_key(Q::Operation, OpVerb::Sync), None);
    assert_eq!(
        option_key(Q::ReadOnly, OpVerb::Copy),
        Some("file_operations.confirm_read_only")
    );
    assert_eq!(
        option_key(Q::System, OpVerb::Delete),
        Some("file_operations.confirm_system")
    );
    assert_eq!(
        option_key(Q::OverwriteNewer, OpVerb::Copy),
        Some("file_operations.confirm_overwrite_newer")
    );
    assert_eq!(
        option_key(Q::OverwriteBackup, OpVerb::Copy),
        Some("file_operations.confirm_overwrite_newer")
    );
    assert_eq!(
        option_key(Q::ReplaceDuringMove, OpVerb::Move),
        Some("file_operations.confirm_replace_during_move")
    );
    assert_eq!(option_key(Q::NoRecycleBin, OpVerb::Delete), None);
    // Every key is a boolean the settings know.
    for kind in [
        Q::Operation,
        Q::ReadOnly,
        Q::System,
        Q::OverwriteNewer,
        Q::ReplaceDuringMove,
    ] {
        for verb in [OpVerb::Copy, OpVerb::Move, OpVerb::Delete] {
            if let Some(key) = option_key(kind, verb) {
                hikasa_session::settings::parse_key(&format!("options.{key}"))
                    .unwrap_or_else(|e| panic!("{key}: {e}"));
            }
        }
    }
}

/// A6.33: a copy that replaces a newer file asks the operation, then the item; each reply does what it says, Yes to All
/// and No to All answer the class, "Do not ask again" with Yes writes that question's key and no other, with No none.
#[test]
fn a6_33_each_question_and_reply() {
    let world = World::new();
    for name in ["a.txt", "b.txt", "c.c"] {
        put(&world, "l", name, "old left\n", 1_000);
        put(&world, "r", name, "newer right\n", 2_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);

    // One row: the operation, then its newer target.
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(top_title(&app).as_deref(), Some("Copy to Right"));
    let d = confirm_dialog(&mut app);
    assert_eq!(d.kind(), Some(QuestionKind::Operation));
    assert_eq!(d.help_topic(), "dialog-confirm");
    assert!(
        d.question().is_some_and(|q| q.starts_with("copy: ")),
        "{:?}",
        d.question()
    );
    assert!(
        d.counts().iter().any(|l| l == "copy: 1 items, 9 bytes"),
        "{:?}",
        d.counts()
    );
    assert_eq!(d.remaining(), 2);
    let roles: Vec<ButtonRole> = d.buttons().iter().map(|b| b.role).collect();
    assert_eq!(
        roles,
        [
            ButtonRole::Yes,
            ButtonRole::No,
            YES_TO_ALL,
            NO_TO_ALL,
            ButtonRole::Cancel
        ]
    );
    assert!(
        d.buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Yes && b.default)
    );
    press(&world, &mut app, ButtonRole::Yes);
    let d = confirm_dialog(&mut app);
    assert_eq!(d.kind(), Some(QuestionKind::OverwriteNewer));
    assert_eq!(d.question(), Some("Overwrite newer a.txt?"));
    assert!(
        d.buttons()
            .iter()
            .any(|b| b.role == ButtonRole::No && b.default),
        "an item's question defaults to No"
    );
    assert_eq!(
        d.do_not_ask_key(),
        Some("file_operations.confirm_overwrite_newer")
    );
    // Ticked with No: nothing is written, the item stays.
    d.ticked = true;
    press(&world, &mut app, ButtonRole::No);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("newer right\n"));
    assert!(app.options().file_operations.confirm_overwrite_newer);
    let options = std::fs::read_to_string(world.path("config/options.toml")).unwrap_or_default();
    assert!(!options.contains("confirm_overwrite_newer"), "{options}");
    assert!(
        logged(&app, LogKind::Summary)
            .iter()
            .any(|t| t.starts_with("copied 0 of 1 items (1 skipped, 0 failed)")),
        "{:?}",
        logged(&app, LogKind::Summary)
    );

    // Yes to All copies all three: each replaces a newer file, one question for the class.
    select(&mut app, view, "b.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    press(&world, &mut app, YES_TO_ALL);
    assert_eq!(
        confirm_dialog(&mut app).kind(),
        Some(QuestionKind::OverwriteNewer)
    );
    // Ticked with Yes: the key of this question is written, and only that one.
    confirm_dialog(&mut app).ticked = true;
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "b.txt").as_deref(), Some("old left\n"));
    assert!(!app.options().file_operations.confirm_overwrite_newer);
    assert!(app.options().file_operations.confirm_copy);
    world.settle(&mut app);
    let options = std::fs::read_to_string(world.path("config/options.toml")).expect("written");
    assert!(
        options.contains("confirm_overwrite_newer = false"),
        "{options}"
    );
    assert!(!options.contains("confirm_copy"), "{options}");

    // Now only the operation is asked; Cancel there writes nothing.
    select(&mut app, view, "c.c");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(confirm_dialog(&mut app).remaining(), 1);
    press(&world, &mut app, ButtonRole::Cancel);
    assert_eq!(read(&world, "r", "c.c").as_deref(), Some("newer right\n"));
    assert!(
        logged(&app, LogKind::Summary)
            .iter()
            .any(|t| t == "Copy to Right: cancelled, nothing was written."),
        "{:?}",
        logged(&app, LogKind::Summary)
    );
    // No to the operation is a Cancel too; No to All there as well.
    for role in [ButtonRole::No, NO_TO_ALL] {
        select(&mut app, view, "c.c");
        app.execute(CommandId::CopyRight);
        world.settle(&mut app);
        press(&world, &mut app, role);
        assert_eq!(dialogs(&app), 0);
        assert_eq!(read(&world, "r", "c.c").as_deref(), Some("newer right\n"));
    }
}

/// A6.33: Esc is Cancel and F1 the dialog's topic, in both dialogs; a dialog closed by the window's close button is a
/// Cancel too.
#[test]
fn a6_33_esc_cancels_and_f1_shows_the_topic() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    key(&world, &mut app, "F1");
    assert_eq!(app.help(), Some("dialog-confirm"));
    assert_eq!(dialogs(&app), 1, "F1 keeps the dialog");
    key(&world, &mut app, "Esc");
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "a.txt"), None);

    app.execute(CommandId::Delete);
    key(&world, &mut app, "F1");
    assert_eq!(app.help(), Some("dialog-delete"));
    key(&world, &mut app, "Esc");
    assert_eq!(dialogs(&app), 0);
    assert!(read(&world, "l", "a.txt").is_some());

    // The window's close button over the question: a Cancel.
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    let w = app.win();
    app.0.close_window(w);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(read(&world, "r", "a.txt"), None);
    // The view is free again.
    assert!(app.enabled(CommandId::CopyRight));
}

// ---- the Delete dialog --------------------------------------------------------------------------------------------

/// A6.33: the Delete dialog's sides, presets, check boxes and Delete Permanently, as a model.
#[test]
fn a6_33_delete_dialog_model() {
    let view = ViewId(7);
    let both = || [Some("a file".to_owned()), Some("a file".to_owned())];
    // A pair on two writable sides: no preset, OK off until a side is chosen.
    let mut d = DeleteDialog::new(view, "a.txt", both(), [true, true], false, false);
    assert_eq!(d.choice(), None);
    let ok = |d: &DeleteDialog| {
        d.buttons()
            .iter()
            .find(|b| b.role == ButtonRole::Ok)
            .is_some_and(|b| b.enabled)
    };
    assert!(!ok(&d));
    assert!(SideChoice::ALL.iter().all(|c| d.enabled(*c)));
    assert!(d.use_bin() && !d.fallback() && d.shows_fallback());
    d.set_choice(SideChoice::Both);
    assert!(ok(&d));
    // A one-sided row: that side is preset, unless the option asks for a choice.
    let one = [None, Some("a file".to_owned())];
    let d = DeleteDialog::new(view, "b.txt", one.clone(), [true, true], false, false);
    assert_eq!(d.choice(), Some(SideChoice::Right));
    assert!(!d.enabled(SideChoice::Left) && !d.enabled(SideChoice::Both));
    let mut d = DeleteDialog::new(view, "b.txt", one, [true, true], false, true);
    assert_eq!(d.choice(), None, "require_side_selection_in_delete");
    assert!(!ok(&d));
    d.set_choice(SideChoice::Left);
    assert_eq!(d.choice(), None, "a side without the row is greyed");
    d.set_choice(SideChoice::Right);
    assert!(ok(&d));
    // A read-only side is greyed; a pair stays without a preset all the same (OK off until a side is chosen).
    let d = DeleteDialog::new(view, "a.txt", both(), [false, true], false, false);
    assert!(!d.enabled(SideChoice::Left) && !d.enabled(SideChoice::Both));
    assert_eq!(d.choice(), None);
    assert!(!ok(&d));
    // A one-sided row on a read-only side: nothing to preset, nothing to choose.
    let d = DeleteDialog::new(
        view,
        "b.txt",
        [Some("a file".to_owned()), None],
        [false, true],
        false,
        false,
    );
    assert_eq!(d.choice(), None);
    assert!(SideChoice::ALL.iter().all(|c| !d.enabled(*c)));
    // The check boxes, and what OK hands back.
    let mut d = DeleteDialog::new(view, "a.txt", both(), [true, true], false, false);
    d.set_choice(SideChoice::Left);
    d.set_fallback(true);
    let service = World::new().service;
    assert_eq!(
        d.on_button(ButtonRole::Ok, &service),
        crate::dialog::DialogStep::Close
    );
    let chosen = d.take_choice().expect("chosen");
    assert_eq!(chosen.sides, SideSet::LEFT);
    assert!(chosen.recycle.use_bin && chosen.recycle.fallback);
    let mut d = DeleteDialog::new(view, "a.txt", both(), [true, true], false, false);
    d.set_choice(SideChoice::Right);
    d.set_fallback(true);
    d.set_use_bin(false);
    d.on_button(ButtonRole::Ok, &service);
    let chosen = d.take_choice().expect("chosen");
    assert!(
        !chosen.recycle.use_bin && !chosen.recycle.fallback,
        "no fallback without the bin"
    );
    // Cancel hands back nothing.
    let mut d = DeleteDialog::new(view, "a.txt", both(), [true, true], false, false);
    d.set_choice(SideChoice::Both);
    d.on_button(ButtonRole::Cancel, &service);
    assert_eq!(d.take_choice(), None);
    // Delete Permanently: the bin off and greyed, no fallback.
    let mut d = DeleteDialog::new(view, "a.txt", both(), [true, true], true, false);
    assert_eq!(d.title(), "Delete Permanently");
    assert!(!d.use_bin() && !d.shows_fallback());
    d.set_use_bin(true);
    d.set_fallback(true);
    assert!(!d.use_bin() && !d.fallback());
    d.set_choice(SideChoice::Left);
    d.on_button(ButtonRole::Ok, &service);
    let chosen = d.take_choice().expect("chosen");
    assert!(!chosen.recycle.use_bin && !chosen.recycle.fallback);
}

/// A6.33: the bin chosen plans a `trash` and never a permanent delete (cancelled here: no test calls the real bin);
/// Delete Permanently plans `remove`; a member, which has no bin, asks before it is deleted for good.
#[test]
fn a6_33_the_bin_is_what_the_dialog_says() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let zip = crate::tests_shell::stored_zip(&[("m.txt", b"member\n"), ("k.txt", b"kept\n")]);
    std::fs::write(world.path("work/l/z.zip"), &zip).expect("zip");
    std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Delete);
    assert!(delete_dialog(&mut app).use_bin());
    press(&world, &mut app, ButtonRole::Ok);
    let counts = confirm_dialog(&mut app).counts().to_vec();
    assert_eq!(counts, ["trash: 1 items"]);
    press(&world, &mut app, ButtonRole::Cancel);
    assert!(read(&world, "l", "a.txt").is_some());
    app.execute(CommandId::PermanentDelete);
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(confirm_dialog(&mut app).counts(), ["remove: 1 items"]);
    press(&world, &mut app, ButtonRole::Cancel);
    assert!(read(&world, "l", "a.txt").is_some());

    // A member with the bin asked for: the no-bin question, and No keeps it.
    app.execute(CommandId::ExpandAll);
    let opened = node_of(&mut app, view, "z.zip/m.txt");
    if opened.is_none() {
        // Archives open on demand: open it in place first.
        select(&mut app, view, "z.zip");
        app.execute(CommandId::Open);
        world.settle(&mut app);
        app.execute(CommandId::ExpandAll);
    }
    select(&mut app, view, "z.zip/m.txt");
    app.execute(CommandId::Delete);
    press(&world, &mut app, ButtonRole::Ok);
    press(&world, &mut app, ButtonRole::Yes);
    let d = confirm_dialog(&mut app);
    assert_eq!(d.kind(), Some(QuestionKind::NoRecycleBin));
    assert_eq!(d.do_not_ask_key(), None, "the bin's question has no key");
    press(&world, &mut app, ButtonRole::No);
    assert_eq!(std::fs::read(world.path("work/l/z.zip")).expect("zip"), zip);
}

// ---- the flow: plan, execute, log, notice, refresh ----------------------------------------------------------------

/// A6.33, A6.34: a copy of a row ends with the row updated, a log entry and an information notice; the view's rows
/// equal a re-run's, and the open folders and the selected row stay, through `renumbered`.
#[test]
fn a6_34_a_copy_refreshes_the_view_as_a_rerun_shows_it() {
    let world = World::new();
    put(&world, "l", "d/x.txt", "new left text\n", 2_000);
    put(&world, "r", "d/x.txt", "old\n", 1_000);
    put(&world, "l", "d/e/y.txt", "y\n", 1_000);
    put(&world, "r", "d/e/y.txt", "y\n", 1_000);
    put(&world, "l", "d/e/f/z.txt", "z\n", 1_000);
    put(&world, "r", "d/e/f/z.txt", "z\n", 1_000);
    put(&world, "l", "top.txt", "t\n", 1_000);
    put(&world, "l", "d/new.txt", "a new file\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    // Open d, d/e and d/e/f by hand (not Expand All).
    for path in ["d", "d/e", "d/e/f"] {
        select(&mut app, view, path);
        app.execute(CommandId::Expand);
    }
    select(&mut app, view, "d/new.txt");
    let before = node_of(&mut app, view, "d/e").expect("d/e");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(dialogs(&app), 0, "nothing more to ask");
    assert_eq!(
        read(&world, "r", "d/new.txt").as_deref(),
        Some("a new file\n")
    );
    // The row is updated, its folder was refreshed: d/e moved, and is still open.
    let after = node_of(&mut app, view, "d/e").expect("d/e");
    assert_ne!(before, after, "the refresh renumbered d's rows");
    let rows = shown(&mut app, view);
    assert!(
        rows.contains(&("d/new.txt".to_owned(), Category::Same)),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|(p, _)| p == "d/e/f/z.txt"),
        "d/e and d/e/f stay open: {rows:?}"
    );
    assert_eq!(selected_path(&mut app, view).as_deref(), Some("d/new.txt"));
    // A re-run of the same comparison with the same folders open shows the same rows.
    let fresh = open(&world, &mut app);
    for path in ["d", "d/e", "d/e/f"] {
        select(&mut app, fresh, path);
        app.execute(CommandId::Expand);
    }
    assert_eq!(shown(&mut app, fresh), rows);
    // The log and the notice.
    let summaries = logged(&app, LogKind::Summary);
    assert!(
        summaries
            .iter()
            .any(|t| t.starts_with("copied 1 of 1 items (0 skipped, 0 failed), 11 bytes")),
        "{summaries:?}"
    );
    let source = app
        .log()
        .entries()
        .find(|e| e.text.starts_with("copied 1 of 1"))
        .map(|e| e.source.clone());
    assert_eq!(source.as_deref(), Some("l ⇄ r"));
    let notes = notices(&app);
    let last = notes.last().expect("a notice");
    assert_eq!(last.0, NoticeKind::Info);
    assert!(
        last.1
            .starts_with("Copy to Right: copied 1 of 1 items (0 skipped, 0 failed), 11 bytes"),
        "{}",
        last.1
    );
}

/// A6.33: a delete of a row by its dialog, without the bin: the file goes, the row goes, a log entry and a notice.
#[test]
fn a6_33_a_delete_of_a_row() {
    let world = World::new();
    put(&world, "l", "keep.txt", "k\n", 1_000);
    put(&world, "r", "keep.txt", "k\n", 1_000);
    put(&world, "r", "gone.txt", "bye\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "gone.txt");
    app.execute(CommandId::Delete);
    let d = delete_dialog(&mut app);
    assert_eq!(d.choice(), Some(SideChoice::Right));
    d.set_use_bin(false);
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(confirm_dialog(&mut app).counts(), ["remove: 1 items"]);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(read(&world, "r", "gone.txt"), None);
    let rows = shown(&mut app, view);
    assert_eq!(rows, [("keep.txt".to_owned(), Category::Same)]);
    assert!(
        logged(&app, LogKind::Summary)
            .iter()
            .any(|t| t.starts_with("deleted 1 of 1 items")),
        "{:?}",
        logged(&app, LogKind::Summary)
    );
    let last = notices(&app).pop().expect("a notice");
    assert_eq!(last.0, NoticeKind::Info);
    assert!(
        last.1.starts_with("Delete: deleted 1 of 1 items"),
        "{}",
        last.1
    );
    assert_eq!(read(&world, "r", "keep.txt").as_deref(), Some("k\n"));
}

/// A6.33: an item that fails (its source changed after the plan) gives a warning notice with *Show log*, which shows the
/// log panel; the log has the item's error.
#[test]
fn a6_33_a_failed_item_gives_a_warning_notice() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    // The source changes while the question waits: its stamp no longer holds (D-163).
    put(&world, "l", "a.txt", "left, and longer\n", 1_500);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(read(&world, "r", "a.txt"), None);
    let errors = logged(&app, LogKind::Error);
    assert!(
        errors
            .iter()
            .any(|t| t.contains("changed since the plan was made")),
        "{errors:?}"
    );
    let (kind, text, actions) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(
        text.starts_with("Copy to Right: copied 0 of 1 items (0 skipped, 1 failed)"),
        "{text}"
    );
    assert_eq!(actions, [NoticeAction::ShowLog(view)]);
    assert!(!folder_model(&mut app, view).show_log());
    let w = app.win();
    let key = app
        .window(w)
        .and_then(|w| w.notices.all().last().map(|n| n.key.clone()))
        .expect("key");
    app.0.notice_action(w, &key, NoticeAction::ShowLog(view));
    assert!(folder_model(&mut app, view).show_log());
}

/// A6.33: Stop cancels the operation's job (deterministic: the job waits at a gate the test opens after Stop); the
/// commands are refused while a job runs; nothing was copied, and the notice says the operation stopped.
#[test]
fn a6_33_stop_cancels_and_commands_wait_for_the_job() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    put(&world, "l", "b.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    // The plan's job waits: the view is busy, Stop is on, the four are off.
    assert!(app.busy());
    assert!(app.enabled(CommandId::Stop));
    for id in [
        CommandId::CopyRight,
        CommandId::Delete,
        CommandId::PermanentDelete,
    ] {
        assert!(!app.enabled(id), "{id:?} while the plan is made");
    }
    assert_eq!(
        app.jobs(app.win())
            .first()
            .map(|j| j.title.clone())
            .as_deref(),
        Some("plan: Copy to Right")
    );
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1);
    set_gate(&g, false);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Yes);
    poll_until(&world, &mut app, |a| {
        a.jobs(w)
            .first()
            .is_some_and(|j| j.title == "Copy to Right")
    });
    assert!(app.enabled(CommandId::Stop));
    assert!(!app.enabled(CommandId::CopyRight));
    app.execute(CommandId::Stop);
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(
        read(&world, "r", "a.txt"),
        None,
        "a stopped copy writes nothing"
    );
    let (kind, text, actions) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning, "{text}");
    assert!(text.starts_with("Copy to Right: stopped;"), "{text}");
    assert_eq!(actions, [NoticeAction::ShowLog(view)]);
    // Free again.
    select(&mut app, view, "b.txt");
    assert!(app.enabled(CommandId::CopyRight));
}

/// A6.33: a plan made on a model that changed under its question does not run (a stale plan is never executed).
#[test]
fn a6_33_a_stale_plan_does_not_run() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    put(&world, "l", "b.txt", "other\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1);
    // The model changes under the question (here: an Ignored mark set through the Service).
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let b = node_of(&mut app, view, "b.txt").expect("b");
    world.service.folder_set_ignored(&model, &[b], true);
    press(&world, &mut app, ButtonRole::Yes);
    assert_eq!(read(&world, "r", "a.txt"), None);
    let (kind, text, _) = notices(&app).pop().expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(text.contains("the comparison changed"), "{text}");
}

/// A6.33: `--nobackups` of a view: its operations keep no backup, whatever `[backups]` says; without it they do.
#[test]
fn a6_33_nobackups_of_a_view() {
    let world = World::with_files(&[(
        "options.toml",
        "[backups]\nback_up_before_copy = true\n[file_operations]\nconfirm_copy = false\n",
    )]);
    put(&world, "l", "a.txt", "new left\n", 2_000);
    put(&world, "r", "a.txt", "old\n", 1_000);
    let mut app = world.app(None);
    let names = |world: &World| {
        let mut names: Vec<String> = std::fs::read_dir(world.path("work/r"))
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let view = open_with(&world, &mut app, |o| o.no_backups = true);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0, "confirm_copy = false: nothing to ask");
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("new left\n"));
    assert_eq!(names(&world), ["a.txt"]);
    // The same without --nobackups: `[backups]` keeps the old file.
    put(&world, "l", "a.txt", "newer left\n", 3_000);
    let plain = open(&world, &mut app);
    select(&mut app, plain, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(read(&world, "r", "a.txt").as_deref(), Some("newer left\n"));
    assert_eq!(names(&world).len(), 2, "{:?}", names(&world));
}

// ---- refresh in the view (A6.34) ----------------------------------------------------------------------------------

/// A6.34: when refreshes leave more records behind than live ones, the view runs a full refresh; the open folders and
/// the selection stay, by path.
#[test]
fn a6_34_garbage_above_half_the_arena_runs_a_full_refresh() {
    let world = World::with_files(&[("options.toml", "[file_operations]\nconfirm_copy = false\n")]);
    for k in 0..40 {
        put(&world, "l", &format!("f{k:02}.txt"), "l\n", 1_000);
    }
    put(&world, "l", "d/inner.txt", "i\n", 1_000);
    std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    let first = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let mut rerun = false;
    for k in 0..4 {
        select(&mut app, view, &format!("f{k:02}.txt"));
        app.execute(CommandId::CopyRight);
        world.settle(&mut app);
        let now = Arc::clone(
            folder_model(&mut app, view)
                .prepared()
                .expect("prepared")
                .model(),
        );
        if !Arc::ptr_eq(&first, &now) {
            rerun = true;
            let read = now.read();
            assert_eq!(read.garbage(), 0, "a re-run starts compact");
            drop(read);
            assert_eq!(
                selected_path(&mut app, view).as_deref(),
                Some(format!("f{k:02}.txt").as_str())
            );
            let rows = shown(&mut app, view);
            assert!(
                rows.iter().any(|(p, _)| p == "d/inner.txt"),
                "d stays open: {rows:?}"
            );
            break;
        }
        let read = now.read();
        assert!(
            read.garbage() <= read.len() / 2,
            "a refresh with garbage below half keeps the model"
        );
    }
    assert!(
        rerun,
        "the garbage passed half the arena within four copies"
    );
}

/// A6.34: Expand on a folder that was not scanned (auto-scan off) refreshes it, then opens it.
#[test]
fn a6_34_expand_on_a_not_scanned_folder() {
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
    select(&mut app, view, "a/b");
    let scan = |app: &mut AppCore| {
        let id = node_of(app, view, "a/b").expect("a/b");
        let m = folder_model(app, view);
        let model = Arc::clone(m.prepared().expect("prepared").model());
        let read = model.read();
        read.node(id).expect("node").scan
    };
    assert_eq!(scan(&mut app), hikasa_folder::ScanStatus::NotScanned);
    assert!(node_of(&mut app, view, "a/b/x.txt").is_none());
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    assert_eq!(scan(&mut app), hikasa_folder::ScanStatus::Scanned);
    let rows = shown(&mut app, view);
    assert!(rows.iter().any(|(p, _)| p == "a/b/x.txt"), "{rows:?}");
}

/// Opens `work/l` against `work/r` with folder rules patched by `patch`.
pub(crate) fn open_folder_with_rules(
    world: &World,
    app: &mut AppCore,
    patch: impl FnOnce(&mut hikasa_folder::FolderRulesPatch),
) -> ViewId {
    let (left, right) = (world.path("work/l"), world.path("work/r"));
    let mut request = hikasa_session::api::FolderCompareRequest {
        left: left.clone().into_os_string(),
        right: right.clone().into_os_string(),
        ..hikasa_session::api::FolderCompareRequest::default()
    };
    patch(&mut request.patch);
    let view = ViewRequest::Folder {
        request,
        display: hikasa_folder::DisplayOptions {
            filter: hikasa_folder::FolderDisplayFilter::All,
            folders: hikasa_folder::FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
        },
        expand_all: false,
    };
    let target = app.open_target();
    let id = app.open_local(
        ViewKind::FolderCompare,
        view,
        &OpenOptions::default(),
        vec![left.display().to_string(), right.display().to_string()],
        target,
    );
    world.settle(app);
    id
}

/// A6.34: FC-012: with `expand_on_load` the folders open after the run; with `expand_differences_only` only those with a
/// difference below them.
#[test]
fn a6_34_expanded_on_load() {
    let world = World::new();
    put(&world, "l", "same/s.txt", "s\n", 1_000);
    put(&world, "r", "same/s.txt", "s\n", 1_000);
    put(&world, "l", "diff/d.txt", "left\n", 1_000);
    put(&world, "r", "diff/d.txt", "right!\n", 1_000);
    let mut app = world.app(None);
    let view = open_folder_with_rules(&world, &mut app, |p| p.expand_on_load = Some(true));
    let rows = shown(&mut app, view);
    assert!(rows.iter().any(|(p, _)| p == "same/s.txt"), "{rows:?}");
    assert!(rows.iter().any(|(p, _)| p == "diff/d.txt"), "{rows:?}");
    let only = open_folder_with_rules(&world, &mut app, |p| {
        p.expand_differences_only = Some(true);
    });
    let rows = shown(&mut app, only);
    assert!(!rows.iter().any(|(p, _)| p == "same/s.txt"), "{rows:?}");
    assert!(rows.iter().any(|(p, _)| p == "diff/d.txt"), "{rows:?}");
}

/// The content result of a row.
pub(crate) fn content_of(app: &mut AppCore, view: ViewId, path: &str) -> Option<ContentKind> {
    let id = node_of(app, view, path)?;
    let m: &mut FolderViewModel = folder_model(app, view);
    let model = Arc::clone(m.prepared()?.model());
    let read = model.read();
    read.node(id)?.content.map(|c| c.kind)
}

/// A6.34: FC-029(c): the result of a child Text Compare view goes to its row; a pair changed meanwhile keeps its own.
#[test]
fn a6_34_a_child_result_updates_its_row() {
    let world = World::new();
    put(&world, "l", "c.txt", "one line\n", 1_000);
    put(&world, "r", "c.txt", "one line\r\n", 1_000);
    put(&world, "l", "d.txt", "two line\n", 1_000);
    put(&world, "r", "d.txt", "two line\r\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert_eq!(content_of(&mut app, view, "c.txt"), None);
    select(&mut app, view, "c.txt");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    let child = app.active_tab().map(|t| t.id).expect("child");
    let verdict = crate::tests::text_model(&mut app, child)
        .response()
        .map(|r| r.comparison.verdict(false));
    assert_eq!(verdict, Some(ContentResult::RulesSame));
    assert_eq!(
        content_of(&mut app, view, "c.txt"),
        Some(ContentKind::RulesSame),
        "the child's result is the row's"
    );
    // Changed meanwhile: the right side grows and its folder is refreshed before the child's result arrives.
    let index = app.tabs().iter().position(|t| t.id == view).expect("tab");
    app.activate(index);
    select(&mut app, view, "d.txt");
    app.execute(CommandId::Open);
    put(&world, "r", "d.txt", "two lines, longer now\r\n", 2_000);
    let service = world.service.clone();
    folder_model(&mut app, view).refresh_for_test(&service, vec![NodeId::ROOT]);
    world.settle(&mut app);
    let child = app.active_tab().map(|t| t.id).expect("child");
    assert!(
        matches!(
            app.tab(child).and_then(|t| t.model()),
            Some(ViewModel::Text(_))
        ),
        "the child opened"
    );
    assert_eq!(
        content_of(&mut app, view, "d.txt"),
        None,
        "a stale child result does not reach a changed row"
    );
    assert!(!folder_model(&mut app, view).has_child(child));
}
