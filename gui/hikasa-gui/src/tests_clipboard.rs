//! M2 W9c acceptance A9.20 (GUI, D-252): `Copy` puts the focus sides' local items on the GUI's own clipboard as a file
//! list in tree order and leaves members out with the message; `Paste` of a file list and of text lines is A9.19's
//! drop; `SelectAll` selects every shown row; `CopyFilenames` writes the specs one a line with the platform's line
//! end; while a text field has the keyboard, Ctrl+C/X/V/A never reach the commands (A4.3's dispatch extended). A
//! fixed clipboard stands in for the system's.

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::Side;
use hikasa_session::clipboard::{ClipboardProvider, FixedClipboard};
use hikasa_session::settings::Shortcut;

use crate::commands::{CommandId, FocusArea};
use crate::drop_dialog::DropDialog;
use crate::folder_view::pane::NOT_LOCAL;
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{node_of, open, put, select};
use crate::window::{CommandArg, Invocation};

fn fixture(world: &World) -> (TestApp, hikasa_session::ipc::ViewId, Arc<FixedClipboard>) {
    put(world, "l", "a.txt", "left a\n", 3_000);
    put(world, "r", "a.txt", "right a\n", 1_000);
    put(world, "l", "b.txt", "only left\n", 1_000);
    put(world, "r", "d/c.txt", "c\n", 1_000);
    put(world, "l", "d/c.txt", "c\n", 1_000);
    let zip = crate::tests_shell::stored_zip(&[("m.txt", b"member\n")]);
    std::fs::write(world.path("work/l/z.zip"), &zip).expect("zip");
    std::fs::write(world.path("work/r/z.zip"), &zip).expect("zip");
    let mut app = world.app(None);
    let fixed = Arc::new(FixedClipboard::text("unused"));
    app.0.set_script_clipboard(Arc::clone(&fixed) as _);
    let view = open(world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    (app, view, fixed)
}

/// Selects the rows at `paths` on `sides` (`None`: every side each has).
fn select_rows(app: &mut TestApp, view: hikasa_session::ipc::ViewId, paths: &[&str]) {
    let nodes: Vec<_> = paths
        .iter()
        .map(|p| node_of(app, view, p).unwrap_or_else(|| panic!("no row {p}")))
        .collect();
    folder_model(app, view).with_selection(|sel, _, present| {
        sel.items = Default::default();
        for node in &nodes {
            sel.items.insert(*node, present(*node));
        }
        sel.current = nodes.first().copied();
        sel.moves += 1;
    });
}

fn sc(text: &str) -> Shortcut {
    Shortcut::parse(text).expect("a shortcut")
}

#[test]
fn a9_20_copy_puts_local_items_in_tree_order() {
    let world = World::new();
    let (mut app, view, fixed) = fixture(&world);
    select_rows(&mut app, view, &["b.txt", "a.txt", "d/c.txt"]);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    let l = |rel: &str| world.path(&format!("work/l/{rel}"));
    let r = |rel: &str| world.path(&format!("work/r/{rel}"));
    // Tree order (folders first, as the tree shows them), the left side first per row; both sides are the focus
    // sides.
    assert_eq!(
        fixed.file_lists(),
        vec![vec![
            l("d/c.txt"),
            r("d/c.txt"),
            l("a.txt"),
            r("a.txt"),
            l("b.txt")
        ]]
    );
    // The clipboard now holds the files: a Paste reads them back (A9.19's drop).
    assert_eq!(fixed.read_files().expect("files").len(), 5);
    // A pane's context menu carries its side: the right items only.
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(CommandId::Copy, CommandArg::Side(Side::Right)),
    );
    world.settle(&mut app);
    assert_eq!(fixed.file_lists()[1], vec![r("d/c.txt"), r("a.txt")]);
}

#[test]
fn a9_20_members_are_left_out_with_the_message() {
    let world = World::new();
    let (mut app, view, fixed) = fixture(&world);
    select(&mut app, view, "z.zip");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    select_rows(&mut app, view, &["a.txt", "z.zip/m.txt"]);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    assert_eq!(
        fixed.file_lists(),
        vec![vec![world.path("work/l/a.txt"), world.path("work/r/a.txt")]]
    );
    let message = app.tabs()[app.active()].message.clone().expect("a message");
    assert_eq!(message.text, NOT_LOCAL);
    // Only members: the message and nothing written.
    select_rows(&mut app, view, &["z.zip/m.txt"]);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    assert_eq!(fixed.file_lists().len(), 1);
}

#[test]
fn a9_20_paste_of_files_and_of_text_lines_is_a_drop() {
    let world = World::new();
    let (mut app, view, fixed) = fixture(&world);
    let outside: Vec<PathBuf> = ["x.txt", "y.txt"]
        .iter()
        .map(|n| world.write(&format!("work/src/{n}"), "new\n"))
        .collect();
    // A file list into the current row's folder (`d` on the left).
    select_rows(&mut app, view, &["d"]);
    folder_model(&mut app, view).with_selection(|sel, _, _| {
        sel.focus = hikasa_folder::select::SideSet::LEFT;
    });
    fixed.set_files(outside.clone());
    app.execute(CommandId::Paste);
    world.settle(&mut app);
    let w = app.win();
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<DropDialog>())
        .expect("the drop's prompt");
    assert_eq!(d.into.side, Side::Left);
    assert_eq!(d.into.shown, world.path("work/l/d").display().to_string());
    assert_eq!(
        d.sources,
        outside
            .iter()
            .map(|p| p.clone().into_os_string())
            .collect::<Vec<_>>()
    );
    crate::tests_folder_ops::press(&world, &mut app, crate::dialog::ButtonRole::Other(1));
    world.settle(&mut app);
    assert_eq!(
        crate::tests_folder_ops::read(&world, "l", "d/x.txt").as_deref(),
        Some("new\n")
    );
    // Text: its lines that are specs (not `-`, not `clipboard://`), from the pane a context menu names.
    let text = format!(
        "{}\n\n-\nclipboard://\n{}\n",
        outside[0].display(),
        outside[1].display()
    );
    fixed.set_text(text);
    app.0.execute(
        w,
        Invocation::with(CommandId::Paste, CommandArg::Side(Side::Right)),
    );
    world.settle(&mut app);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<DropDialog>())
        .expect("the drop's prompt");
    assert_eq!(d.into.side, Side::Right);
    assert_eq!(d.sources.len(), 2);
    crate::tests_folder_ops::press(&world, &mut app, crate::dialog::ButtonRole::Cancel);
    // Nothing to paste: a message, no prompt.
    fixed.set_files(Vec::new());
    app.0.execute(
        w,
        Invocation::with(CommandId::Paste, CommandArg::Side(Side::Right)),
    );
    world.settle(&mut app);
    assert_eq!(
        app.tabs()[app.active()]
            .message
            .as_ref()
            .map(|m| m.text.clone()),
        Some(crate::app::pane::NOTHING_TO_PASTE.to_owned())
    );
}

#[test]
fn a9_20_select_all_and_copy_filename() {
    let world = World::new();
    let (mut app, view, fixed) = fixture(&world);
    app.execute(CommandId::SelectAll);
    let m = folder_model(&mut app, view);
    let rows = m.rows().len();
    assert_eq!(m.selection().items.len(), rows, "every shown row");
    select_rows(&mut app, view, &["a.txt", "b.txt"]);
    app.execute(CommandId::CopyFilenames);
    world.settle(&mut app);
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    let spec = |side: &str, rel: &str| {
        hikasa_core::Spec::Local(
            hikasa_core::LocalPath::new(world.path(&format!("work/{side}/{rel}"))).expect("local"),
        )
        .canonical()
    };
    let expected = [spec("l", "a.txt"), spec("r", "a.txt"), spec("l", "b.txt")].join(eol);
    let payloads = fixed.payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].text, expected);
    assert_eq!(payloads[0].html, None);
}

/// A4.3's dispatch, extended (D-252): the folder pane's keys reach Copy, Paste, Select All; a text field keeps them.
#[test]
fn a9_20_a_text_field_keeps_its_editing_keys() {
    let world = World::new();
    let (mut app, view, _) = fixture(&world);
    select_rows(&mut app, view, &["a.txt"]);
    let w = app.win();
    let table: &[(&str, FocusArea, bool, Option<CommandId>)] = &[
        ("Ctrl+C", FocusArea::ViewMain, false, Some(CommandId::Copy)),
        ("Ctrl+V", FocusArea::ViewMain, false, Some(CommandId::Paste)),
        (
            "Ctrl+A",
            FocusArea::ViewMain,
            false,
            Some(CommandId::SelectAll),
        ),
        ("Ctrl+C", FocusArea::Root, false, Some(CommandId::Copy)),
        ("Ctrl+C", FocusArea::TextField, true, None),
        ("Ctrl+V", FocusArea::TextField, true, None),
        ("Ctrl+X", FocusArea::TextField, true, None),
        ("Ctrl+A", FocusArea::TextField, true, None),
        ("Ctrl+C", FocusArea::PathField, true, None),
        ("Ctrl+A", FocusArea::HomeSearch, true, None),
        // A shortcut that is not an editing key still reaches the window from a text field.
        (
            "Ctrl+F5",
            FocusArea::TextField,
            true,
            Some(CommandId::FullRefresh),
        ),
    ];
    for (key, area, text_focus, expected) in table {
        app.0.set_text_focus(w, *text_focus);
        assert_eq!(
            app.0.dispatch_keys(w, &[sc(key)], *area),
            *expected,
            "{key} at {area:?}"
        );
    }
    // With a text field focused the pane's Edit entries are off in the menus too.
    app.0.set_text_focus(w, true);
    for id in [CommandId::Copy, CommandId::Paste, CommandId::SelectAll] {
        assert!(!app.enabled(id), "{id:?}");
    }
    app.0.set_text_focus(w, false);
    for id in [CommandId::Copy, CommandId::Paste, CommandId::SelectAll] {
        assert!(app.enabled(id), "{id:?}");
    }
}
