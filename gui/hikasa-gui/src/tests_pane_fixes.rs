//! M2 W9c after the review: the panes' drops, pastes, menus, keys and the Filters edit as the review's findings ask
//! (F1–F17 by its numbers), and the guards its mutations found untested (F6). Through the models, a fixed clipboard
//! and temporary trees, as `tests_dnd` and `tests_clipboard`.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_folder::ops::ReadOnlySides;
use hikasa_folder::select::SideSet;
use hikasa_session::clipboard::FixedClipboard;

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::dnd::{DropTarget, RowDrag, RowTarget};
use crate::drop_dialog::{DropDialog, OPEN_NEW};
use crate::folder_view::WritePolicy;
use crate::folder_view::pane::read_only_message;
use crate::op_dialogs::OpDialog;
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{node_of, open, open_with, put, select};
use crate::window::{CommandArg, Invocation};

fn drop_dialog(app: &mut TestApp) -> Option<&mut DropDialog> {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<DropDialog>())
}

fn press(world: &World, app: &mut TestApp, role: ButtonRole) {
    crate::tests_folder_ops::press(world, app, role);
    world.settle(app);
}

/// The prompt's plans are made (its job polled to the end).
fn planned(world: &World, app: &mut TestApp) {
    crate::tests_op_dialogs::planned(world, app);
}

fn enabled(d: &dyn DialogModel, role: ButtonRole) -> bool {
    d.buttons().iter().any(|b| b.role == role && b.enabled)
}

fn exists(world: &World, rel: &str) -> bool {
    world.path(rel).exists()
}

/// A tree with `b.txt` on the left only and a pair `a.txt`, a folder `d` on both sides; a fixed clipboard.
fn fixture(world: &World) -> (TestApp, Arc<FixedClipboard>) {
    put(world, "l", "a.txt", "left a\n", 3_000);
    put(world, "r", "a.txt", "right a\n", 1_000);
    put(world, "l", "b.txt", "only left\n", 1_000);
    put(world, "l", "d/c.txt", "c\n", 1_000);
    put(world, "r", "d/c.txt", "c\n", 1_000);
    let mut app = world.app(None);
    let fixed = Arc::new(FixedClipboard::text("unused"));
    app.0.set_script_clipboard(Arc::clone(&fixed) as _);
    (app, fixed)
}

// ---- F1: a move never takes items from a read-only side ---------------------------------------------------------

/// The review's probe: the left side read-only, its `b.txt` copied and pasted on the right; *Move* is off with note
/// 5's message and pressed moves nothing; *Copy* copies.
#[test]
fn f1_a_paste_from_a_read_only_side_copies_only() {
    let world = World::new();
    let (mut app, fixed) = fixture(&world);
    let view = open_with(&world, &mut app.0, |o| o.read_only.left = true);
    select(&mut app.0, view, "b.txt");
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    assert_eq!(fixed.file_lists().len(), 1);
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(CommandId::Paste, CommandArg::Side(Side::Right)),
    );
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("the drop's prompt");
    let message = read_only_message(Side::Left);
    assert_eq!(d.move_refused.as_deref(), Some(message.as_str()));
    assert_eq!(message, "The left side is read-only (Disable editing).");
    assert!(!d.show_move);
    assert!(enabled(d, ButtonRole::Other(1)));
    assert!(!enabled(d, ButtonRole::Other(2)));
    // The move's preview is the reason.
    d.show_move = true;
    assert_eq!(d.shown().cloned().map(|p| p.err()), Some(Some(message)));
    press(&world, &mut app, ButtonRole::Other(2));
    assert!(drop_dialog(&mut app).is_some(), "Move does nothing");
    assert!(exists(&world, "work/l/b.txt") && !exists(&world, "work/r/b.txt"));
    drop_dialog(&mut app).expect("prompt").show_move = false;
    press(&world, &mut app, ButtonRole::Other(1));
    assert!(drop_dialog(&mut app).is_none());
    assert!(exists(&world, "work/l/b.txt") && exists(&world, "work/r/b.txt"));
}

/// Each source is mapped onto the read-only bases by its components: the base itself, a path through `..`, not a
/// sibling whose name starts the same; a member base by its archive; the other side's items are free.
#[test]
fn f1_sources_are_mapped_onto_the_read_only_bases() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let view = open_with(&world, &mut app.0, |o| o.read_only.left = true);
    let m = folder_model(&mut app.0, view);
    let refused = |m: &crate::folder_view::FolderViewModel, rel: &str| {
        m.move_refused(&[world.path(rel).into_os_string()])
            .is_some()
    };
    assert!(refused(m, "work/l"));
    assert!(refused(m, "work/l/b.txt"));
    assert!(refused(m, "work/l/d/c.txt"));
    assert!(refused(m, "work/r/../l/b.txt"));
    assert!(!refused(m, "work/r/a.txt"));
    assert!(!refused(m, "work/lx/b.txt"));
    assert!(!refused(m, "work"));
    // One refused source refuses the move of all.
    assert!(
        m.move_refused(&[
            world.path("work/r/a.txt").into_os_string(),
            world.path("work/l/./b.txt").into_os_string(),
        ])
        .is_some()
    );
    #[cfg(any(windows, target_os = "macos"))]
    assert!(refused(m, "WORK/L/B.TXT") || refused(m, "work/L/b.txt"));
    // Writable sides refuse nothing.
    m.set_write_policy(WritePolicy::default());
    assert!(!refused(m, "work/l/b.txt"));
}

/// The move is checked again when its button is pressed: a side made read-only while the prompt was open moves
/// nothing, and the view says why.
#[test]
fn f1_a_move_is_checked_again_before_it_runs() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let view = open(&world, &mut app.0);
    let w = app.win();
    app.0.drop_paths_moving(
        w,
        DropTarget::PaneTree {
            side: Side::Right,
            folder: None,
        },
        vec![world.path("work/l/b.txt")],
        true,
    );
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert!(d.show_move && enabled(d, ButtonRole::Other(2)));
    folder_model(&mut app.0, view).set_write_policy(WritePolicy {
        read_only: ReadOnlySides {
            left: true,
            right: false,
        },
        no_backups: false,
    });
    press(&world, &mut app, ButtonRole::Other(2));
    assert!(exists(&world, "work/l/b.txt") && !exists(&world, "work/r/b.txt"));
    assert_eq!(
        app.tabs()[app.active()]
            .message
            .as_ref()
            .map(|m| m.text.clone()),
        Some(read_only_message(Side::Left))
    );
    assert!(
        folder_model(&mut app.0, view).idle(),
        "the view is free again"
    );
}

// ---- F4: a drop with no place in the window asks for the pane ----------------------------------------------------

#[test]
fn f4_a_drop_with_no_place_asks_for_the_pane() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let new = world.write("work/src/new.txt", "new\n");
    let view = open(&world, &mut app.0);
    app.execute(CommandId::ExpandAll);
    // The focus pane is the left one, its current row the folder `d`.
    select(&mut app.0, view, "d");
    folder_model(&mut app.0, view).with_selection(|sel, _, _| sel.focus = SideSet::LEFT);
    let w = app.win();
    app.0.drop_paths(w, DropTarget::PaneAsk, vec![new.clone()]);
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("the prompt asks");
    let sides = d.sides.clone().expect("the panes to choose from");
    let into = d.into.clone().expect("a pane");
    assert_eq!(into.side, Side::Left);
    assert_eq!(into.shown, world.path("work/l/d").display().to_string());
    assert_eq!(
        sides[1].as_ref().map(|i| i.shown.clone()),
        Ok(world.path("work/r/d").display().to_string())
    );
    assert!(d.open_new && d.buttons().iter().any(|b| b.role == OPEN_NEW));
    // The right pane chosen: planned again, copied there.
    d.choose_side(Side::Right);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(d.into.as_ref().map(|i| i.side), Some(Side::Right));
    assert!(d.message().contains("work"), "{}", d.message());
    press(&world, &mut app, ButtonRole::Other(1));
    assert!(exists(&world, "work/r/d/new.txt"));
    assert!(!exists(&world, "work/l/d/new.txt"));
}

/// Two paths: *Open as new comparison* is W4's window rule (a comparison of the two opens); three paths are not
/// offered it; a side that cannot take them is shown with its reason and the other side is chosen.
#[test]
fn f4_open_as_new_comparison_and_sides_that_cannot_take_them() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let (x, y) = (
        world.write("work/p/x.txt", "x\n"),
        world.write("work/q/x.txt", "y\n"),
    );
    let view = open_with(&world, &mut app.0, |o| o.read_only.left = true);
    let w = app.win();
    app.0
        .drop_paths(w, DropTarget::PaneAsk, vec![x.clone(), y.clone()]);
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    let sides = d.sides.clone().expect("sides");
    assert_eq!(sides[0], Err(read_only_message(Side::Left)));
    assert_eq!(d.into.as_ref().map(|i| i.side), Some(Side::Right));
    let tabs = app.tabs().len();
    press(&world, &mut app, OPEN_NEW);
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
    }
    assert!(drop_dialog(&mut app).is_none());
    assert_eq!(app.tabs().len(), tabs + 1, "a new comparison of the two");
    assert!(folder_model(&mut app.0, view).idle());
    // Three paths: no such button.
    let z = world.write("work/p/z.txt", "z\n");
    let first = view;
    let index = app
        .tabs()
        .iter()
        .position(|t| t.id == first)
        .expect("the first tab");
    app.0.activate(w, index);
    app.0.drop_paths(w, DropTarget::PaneAsk, vec![x, y, z]);
    world.settle(&mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert!(!d.open_new && !d.buttons().iter().any(|b| b.role == OPEN_NEW));
    press(&world, &mut app, ButtonRole::Cancel);
    // Neither side can take them: only Open as new comparison and Cancel.
    folder_model(&mut app.0, view).set_write_policy(WritePolicy {
        read_only: ReadOnlySides {
            left: true,
            right: true,
        },
        no_backups: false,
    });
    app.0
        .drop_paths(w, DropTarget::PaneAsk, vec![world.path("work/p/x.txt")]);
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert!(d.into.is_none());
    assert!(d.message().starts_with("Neither pane"), "{}", d.message());
    assert!(!enabled(d, ButtonRole::Other(1)) && !enabled(d, ButtonRole::Other(2)));
    assert!(enabled(d, OPEN_NEW));
    press(&world, &mut app, ButtonRole::Cancel);
}

// ---- F5: rows dragged onto a folder carry the folder's spec ------------------------------------------------------

/// A folder whose name ends in a space: Copy to Folder takes it as it is (never trimmed nor read back), so the rows
/// land in it and no other folder is made.
#[cfg(not(windows))]
#[test]
fn f5_rows_dragged_onto_a_folder_keep_its_spec() {
    let world = World::new();
    put(&world, "l", "a.txt", "left a\n", 3_000);
    put(&world, "r", "a.txt", "right a\n", 1_000);
    put(&world, "r", "f /z.txt", "z\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app.0);
    app.execute(CommandId::ExpandAll);
    select(&mut app.0, view, "a.txt");
    let rows = folder_model(&mut app.0, view).selection().items.clone();
    let folder = node_of(&mut app.0, view, "f ").expect("the folder `f `");
    let w = app.win();
    app.0.drop_rows(
        w,
        &RowDrag {
            from: Some(Side::Left),
            rows,
        },
        RowTarget::Pane {
            side: Side::Right,
            folder: Some(folder),
        },
        false,
    );
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<OpDialog>())
        .expect("Copy to Folder");
    let resolved = d.resolved_target().expect("a target").expect("resolved");
    assert_eq!(
        hikasa_session::api::spec_os_string(&resolved),
        world.path("work/r/f ").into_os_string()
    );
    assert!(
        matches!(d.form(), crate::op_dialogs::OpForm::CopyTo { target, .. } if target.ends_with("f "))
    );
    crate::tests_op_dialogs::start_and_confirm(&world, &mut app);
    assert!(exists(&world, "work/r/f /a.txt"));
    assert!(!exists(&world, "work/r/f"), "no folder `f` made");
}

/// The field edited: the text is read as typed again (trimmed, parsed).
#[test]
fn f5_an_edited_target_is_read_as_typed() {
    let world = World::new();
    put(&world, "l", "a.txt", "left a\n", 3_000);
    put(&world, "r", "e/z.txt", "z\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app.0);
    select(&mut app.0, view, "a.txt");
    let rows = folder_model(&mut app.0, view).selection().items.clone();
    let folder = node_of(&mut app.0, view, "e").expect("e");
    let w = app.win();
    app.0.drop_rows(
        w,
        &RowDrag {
            from: Some(Side::Left),
            rows,
        },
        RowTarget::Pane {
            side: Side::Right,
            folder: Some(folder),
        },
        false,
    );
    world.settle(&mut app);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<OpDialog>())
        .expect("Copy to Folder");
    let other = world.path("work/r/g");
    d.set_target(&format!("  {}  ", other.display()));
    let resolved = d.resolved_target().expect("a target").expect("resolved");
    assert_eq!(
        hikasa_session::api::spec_os_string(&resolved),
        other.into_os_string()
    );
}

/// A folder whose name is not valid Unicode is refused with a message (its text could not be read back).
#[cfg(target_os = "linux")]
#[test]
fn f5_a_folder_that_is_not_unicode_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let world = World::new();
    put(&world, "l", "a.txt", "left a\n", 3_000);
    let odd = world
        .path("work/r")
        .join(std::ffi::OsStr::from_bytes(b"f\xff"));
    std::fs::create_dir_all(&odd).expect("mkdir");
    let mut app = world.app(None);
    let view = open(&world, &mut app.0);
    select(&mut app.0, view, "a.txt");
    let rows = folder_model(&mut app.0, view).selection().items.clone();
    let folder = {
        let m = folder_model(&mut app.0, view);
        let model = Arc::clone(m.prepared().expect("p").model());
        let read = model.read();
        read.preorder()
            .map(|(id, _)| id)
            .find(|&id| read.path(id).as_str().starts_with('f'))
            .expect("the folder")
    };
    let w = app.win();
    let before = crate::tests_folder_ops::dialogs(&app);
    app.0.drop_rows(
        w,
        &RowDrag {
            from: Some(Side::Left),
            rows,
        },
        RowTarget::Pane {
            side: Side::Right,
            folder: Some(folder),
        },
        false,
    );
    world.settle(&mut app);
    assert_eq!(crate::tests_folder_ops::dialogs(&app), before);
    assert_eq!(
        app.tabs()[app.active()]
            .message
            .as_ref()
            .map(|m| m.text.clone()),
        Some(crate::app::pane::LOSSY_FOLDER.to_owned())
    );
}

// ---- F8, F9: Shift at a drop, a drop on a file row ---------------------------------------------------------------

/// Shift held at the drop shows the move first and makes it the default (note 4); not when the move is refused.
#[test]
fn f8_shift_at_a_drop_shows_the_move_first() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let new = world.write("work/src/new.txt", "new\n");
    let _view = open(&world, &mut app.0);
    let w = app.win();
    let target = DropTarget::PaneTree {
        side: Side::Right,
        folder: None,
    };
    for (moving, source) in [(true, new.clone()), (false, new.clone())] {
        app.0.drop_paths_moving(w, target, vec![source], moving);
        world.settle(&mut app);
        planned(&world, &mut app);
        let d = drop_dialog(&mut app).expect("prompt");
        assert_eq!(d.show_move, moving);
        let default = d.buttons().into_iter().find(|b| b.default).map(|b| b.role);
        assert_eq!(default, Some(ButtonRole::Other(if moving { 2 } else { 1 })));
        press(&world, &mut app, ButtonRole::Cancel);
    }
}

/// Paths dropped on a file row go into that file's folder, as a paste from it does.
#[test]
fn f9_a_drop_on_a_file_row_goes_into_its_folder() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let new = world.write("work/src/new.txt", "new\n");
    let view = open(&world, &mut app.0);
    app.execute(CommandId::ExpandAll);
    let file = node_of(&mut app.0, view, "d/c.txt").expect("d/c.txt");
    let row = folder_model(&mut app.0, view)
        .rows()
        .iter()
        .position(|r| r.node == file)
        .expect("shown");
    let folder = folder_model(&mut app.0, view).drop_row(row);
    assert_eq!(folder, Some(file));
    let w = app.win();
    app.0.drop_paths(
        w,
        DropTarget::PaneTree {
            side: Side::Left,
            folder,
        },
        vec![new],
    );
    world.settle(&mut app);
    planned(&world, &mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(
        d.into.as_ref().map(|i| i.shown.clone()),
        Some(world.path("work/l/d").display().to_string())
    );
    press(&world, &mut app, ButtonRole::Other(1));
    assert!(exists(&world, "work/l/d/new.txt"));
}

// ---- F7, F12: what a text paste takes; clipboard writes in order -------------------------------------------------

/// Pasted text: only absolute local paths and `file://` URIs are sources; prose or a relative path is "nothing to
/// paste" (not a prompt whose plan fails), trimmed lines are paths.
#[test]
fn f7_pasted_text_takes_absolute_paths_and_file_uris_only() {
    let world = World::new();
    let (mut app, fixed) = fixture(&world);
    let new = world.write("work/src/new 1.txt", "new\n");
    let _view = open(&world, &mut app.0);
    let w = app.win();
    let paste = |app: &mut TestApp| {
        app.0.execute(
            w,
            Invocation::with(CommandId::Paste, CommandArg::Side(Side::Right)),
        );
        world.settle(app);
    };
    fixed.set_text("Some words someone copied.\nsrc/new 1.txt\nhttp://example.com/a\n");
    paste(&mut app);
    assert!(drop_dialog(&mut app).is_none());
    assert_eq!(
        app.tabs()[app.active()]
            .message
            .as_ref()
            .map(|m| m.text.clone()),
        Some(crate::app::pane::NOTHING_TO_PASTE.to_owned())
    );
    // A path with spaces around it, and the same file as a URI.
    let uri = hikasa_session::host::file_uri(&new);
    fixed.set_text(format!("  {}  \r\n{uri}\n", new.display()));
    paste(&mut app);
    let d = drop_dialog(&mut app).expect("the prompt");
    assert_eq!(
        d.sources,
        [new.clone().into_os_string(), new.into_os_string()]
    );
    press(&world, &mut app, ButtonRole::Cancel);
}

/// A clipboard that logs the kind of each write, in order.
#[derive(Default)]
struct Logged(parking_lot::Mutex<Vec<&'static str>>);

impl hikasa_session::clipboard::ClipboardProvider for Logged {
    fn read(
        &self,
    ) -> Result<
        hikasa_session::clipboard::ClipboardContent,
        hikasa_session::clipboard::ClipboardError,
    > {
        Err(hikasa_session::clipboard::ClipboardError::Empty)
    }

    fn write_payload(
        &self,
        _: &hikasa_session::clipboard::ClipboardPayload,
    ) -> Result<(), hikasa_session::clipboard::ClipboardError> {
        self.0.lock().push("names");
        Ok(())
    }

    fn write_files(
        &self,
        _: &[std::path::PathBuf],
    ) -> Result<(), hikasa_session::clipboard::ClipboardError> {
        std::thread::sleep(std::time::Duration::from_millis(20));
        self.0.lock().push("files");
        Ok(())
    }
}

/// Copy then Copy Filename asked at once: the clipboard ends with the later one, whichever job runs first (an older
/// write that has not begun is skipped; one that has is done before the later one starts).
#[test]
fn f12_the_latest_clipboard_write_wins() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let logged = Arc::new(Logged::default());
    app.0.set_script_clipboard(Arc::clone(&logged) as _);
    let view = open(&world, &mut app.0);
    select(&mut app.0, view, "a.txt");
    for _ in 0..5 {
        app.execute(CommandId::Copy);
        app.execute(CommandId::CopyFilenames);
        world.settle(&mut app);
        assert_eq!(
            logged.0.lock().last(),
            Some(&"names"),
            "{:?}",
            logged.0.lock()
        );
    }
}

// ---- F6: the guards the review's mutations found untested ---------------------------------------------------------

/// A drop's prompt with its plans made (a file dropped on the right base).
fn prompt_on_the_right(world: &World, app: &mut TestApp, source: std::path::PathBuf) {
    let w = app.win();
    app.0.drop_paths(
        w,
        DropTarget::PaneTree {
            side: Side::Right,
            folder: None,
        },
        vec![source],
    );
    world.settle(app);
    planned(world, app);
}

/// A changed overwrite choice: the buttons wait for its plan (M1); Cancel keeps the remembered choice, a run records
/// it (M2); with none remembered (or a word that is none) the choice is Fail (M3).
#[test]
fn f6_the_overwrite_choice_waits_for_its_plan_and_is_remembered_by_a_run() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let new = world.write("work/src/a.txt", "new a\n");
    let _view = open(&world, &mut app.0);
    assert_eq!(
        crate::drop_dialog::overwrite_of(None),
        hikasa_vfs::ops::OverwritePolicy::Fail
    );
    assert_eq!(
        crate::drop_dialog::overwrite_of(Some("no such word")),
        hikasa_vfs::ops::OverwritePolicy::Fail
    );
    prompt_on_the_right(&world, &mut app, new.clone());
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(
        d.overwrite,
        hikasa_vfs::ops::OverwritePolicy::Fail,
        "the default"
    );
    assert!(enabled(d, ButtonRole::Other(1)));
    d.set_overwrite(hikasa_vfs::ops::OverwritePolicy::Always);
    assert!(!enabled(d, ButtonRole::Other(1)), "waits for the new plan");
    assert!(matches!(
        d.on_button(ButtonRole::Other(1), &world.service),
        crate::dialog::DialogStep::Stay
    ));
    assert!(d.take_answer().is_none());
    // Cancel: nothing remembered.
    press(&world, &mut app, ButtonRole::Cancel);
    prompt_on_the_right(&world, &mut app, new.clone());
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(
        d.overwrite,
        hikasa_vfs::ops::OverwritePolicy::Fail,
        "Cancel kept nothing"
    );
    d.set_overwrite(hikasa_vfs::ops::OverwritePolicy::Always);
    planned(&world, &mut app);
    press(&world, &mut app, ButtonRole::Other(1));
    assert_eq!(
        std::fs::read_to_string(world.path("work/r/a.txt"))
            .ok()
            .as_deref(),
        Some("new a\n")
    );
    prompt_on_the_right(&world, &mut app, new);
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(
        d.overwrite,
        hikasa_vfs::ops::OverwritePolicy::Always,
        "a run records it"
    );
    press(&world, &mut app, ButtonRole::Cancel);
}

/// A drop on a folder row the side does not have goes into its nearest folder there, the base here (M5).
#[test]
fn f6_a_drop_on_a_folder_the_side_lacks_goes_up() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    put(&world, "r", "e/z.txt", "z\n", 1_000);
    let new = world.write("work/src/new.txt", "new\n");
    let view = open(&world, &mut app.0);
    let e = node_of(&mut app.0, view, "e").expect("e, on the right only");
    let w = app.win();
    app.0.drop_paths(
        w,
        DropTarget::PaneTree {
            side: Side::Left,
            folder: Some(e),
        },
        vec![new],
    );
    world.settle(&mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    let into = d.into.clone().expect("a pane");
    assert_eq!(into.shown, world.path("work/l").display().to_string());
    assert_eq!(into.folder, None);
    press(&world, &mut app, ButtonRole::Cancel);
}

/// Paste goes into the current row's folder on the focus side (a file's folder, M6); with both sides focused and no
/// pane named it asks for a row instead of guessing (M7).
#[test]
fn f6_where_a_paste_goes() {
    let world = World::new();
    let (mut app, fixed) = fixture(&world);
    let new = world.write("work/src/new.txt", "new\n");
    fixed.set_files(vec![new]);
    let view = open(&world, &mut app.0);
    app.execute(CommandId::ExpandAll);
    select(&mut app.0, view, "d/c.txt");
    folder_model(&mut app.0, view).with_selection(|sel, _, _| sel.focus = SideSet::RIGHT);
    app.execute(CommandId::Paste);
    world.settle(&mut app);
    let d = drop_dialog(&mut app).expect("prompt");
    assert_eq!(
        d.into.as_ref().map(|i| (i.side, i.shown.clone())),
        Some((Side::Right, world.path("work/r/d").display().to_string()))
    );
    press(&world, &mut app, ButtonRole::Cancel);
    folder_model(&mut app.0, view).with_selection(|sel, _, _| sel.focus = SideSet::all());
    app.execute(CommandId::Paste);
    world.settle(&mut app);
    assert!(drop_dialog(&mut app).is_none());
    assert_eq!(
        app.tabs()[app.active()]
            .message
            .as_ref()
            .map(|m| m.text.clone()),
        Some(crate::folder_view::pane::PASTE_WHERE.to_owned())
    );
}

/// A move refreshes the folders its sources left (M21); a copy's failed items go to the log (M22).
#[test]
fn f6_a_move_refreshes_its_sources_and_failures_are_logged() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let view = open(&world, &mut app.0);
    let w = app.win();
    app.0.drop_paths_moving(
        w,
        DropTarget::PaneTree {
            side: Side::Right,
            folder: None,
        },
        vec![world.path("work/l/d/c.txt")],
        true,
    );
    world.settle(&mut app);
    planned(&world, &mut app);
    press(&world, &mut app, ButtonRole::Other(2));
    crate::tests_folder_ops::poll_until(&world, &mut app.0, |app| {
        crate::tests::folder_model(app, view).idle()
    });
    assert!(!exists(&world, "work/l/d/c.txt") && exists(&world, "work/r/c.txt"));
    // The source's folder `d` (not the folder it went into) was read again: `d/c.txt` is on the right only now.
    let b = node_of(&mut app.0, view, "d/c.txt").expect("d/c.txt");
    let sides = {
        let m = folder_model(&mut app.0, view);
        let model = Arc::clone(m.prepared().expect("p").model());
        let read = model.read();
        crate::folder_view::present_sides(&read, b)
    };
    assert_eq!(sides, SideSet::RIGHT, "the source folder was refreshed too");
    // A copy over an existing item with Fail: the failure is logged.
    prompt_on_the_right(&world, &mut app, world.path("work/l/a.txt"));
    press(&world, &mut app, ButtonRole::Other(1));
    crate::tests_folder_ops::poll_until(&world, &mut app.0, |app| {
        crate::tests::folder_model(app, view).idle()
    });
    let errors = crate::tests_folder_ops::logged(&app, hikasa_session::log::LogKind::Error);
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("failed ") && e.contains("a.txt")),
        "{errors:?}"
    );
}

/// Rows dragged with Shift onto a folder: Move to Folder only when it could run (M29: the left side read-only, its
/// item on the left only).
#[test]
fn f6_a_move_to_a_folder_from_a_read_only_side_is_not_offered() {
    let world = World::new();
    let (mut app, _) = fixture(&world);
    let view = open_with(&world, &mut app.0, |o| o.read_only.left = true);
    select(&mut app.0, view, "b.txt");
    let rows = folder_model(&mut app.0, view).selection().items.clone();
    let folder = node_of(&mut app.0, view, "d").expect("d");
    let w = app.win();
    let before = crate::tests_folder_ops::dialogs(&app);
    app.0.drop_rows(
        w,
        &RowDrag {
            from: Some(Side::Left),
            rows,
        },
        RowTarget::Pane {
            side: Side::Right,
            folder: Some(folder),
        },
        true,
    );
    world.settle(&mut app);
    assert_eq!(
        crate::tests_folder_ops::dialogs(&app),
        before,
        "no Move to Folder"
    );
}
