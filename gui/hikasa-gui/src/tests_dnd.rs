//! M2 W9c acceptance A9.19 (D-251): drag and drop through the models. `dnd::decide` and `dnd::decide_rows` for every row
//! of §1.9.16's table; paths from the OS dropped into a folder of a pane: the prompt shows core's plans of a copy and a
//! move, each button runs the plan it showed (the disk equals `hikasa vfs cp|mv` of the same sources on a twin), the
//! overwrite choices, the folder refreshed; rows dragged to the other pane are `CopySide`/`MoveSide` and onto a folder
//! row Copy/Move to Folder prefilled (*relative*); nothing during an operation. Disk trees in a private folder.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::PathStructure;
use hikasa_session::api::VfsOpRequest;
use hikasa_vfs::ops::{CopyOptions, MoveOptions, OverwritePolicy};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::dnd::{
    DropOutcome, DropTarget, FieldId, RowDrag, RowDrop, RowTarget, decide, decide_rows,
};
use crate::drop_dialog::DropDialog;
use crate::op_dialogs::{OpDialog, OpForm};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{dialogs, gate, node_of, notices, open, put, select, set_gate};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

/// Every file and folder below `rel`: relative path → content (`/` for a folder).
fn listing(world: &World, rel: &str) -> BTreeMap<String, String> {
    let root = world.path(rel);
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(&root)
                .expect("below")
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(rel, "/".to_owned());
                stack.push(path);
            } else {
                out.insert(rel, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}

fn drop_dialog(app: &mut TestApp) -> Option<&mut DropDialog> {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<DropDialog>())
}

/// Presses `role` on the top dialog and lets every job end.
fn press(world: &World, app: &mut TestApp, role: ButtonRole) {
    crate::tests_folder_ops::press(world, app, role);
    world.settle(app);
}

#[test]
fn a9_19_decide_follows_the_table() {
    let (a, b, c) = (p("/a"), p("/b"), p("/c"));
    // A path edit or a pane's header: that side's base; two paths on the left: both.
    assert_eq!(
        decide(DropTarget::PanePath(Side::Right), std::slice::from_ref(&a)),
        DropOutcome::Bases(vec![(Side::Right, "/a".into())])
    );
    assert_eq!(
        decide(DropTarget::PanePath(Side::Left), &[a.clone(), b.clone()]),
        DropOutcome::Bases(vec![(Side::Left, "/a".into()), (Side::Right, "/b".into())])
    );
    assert_eq!(
        decide(
            DropTarget::PanePath(Side::Right),
            &[a.clone(), b.clone(), c.clone()]
        ),
        DropOutcome::Bases(vec![(Side::Right, "/a".into())])
    );
    // A pane's tree: a folder row or the empty area takes one, two or three paths.
    let folder = Some(hikasa_folder::NodeId(3));
    for paths in [
        vec![a.clone()],
        vec![a.clone(), b.clone()],
        vec![a.clone(), b.clone(), c.clone()],
    ] {
        for folder in [folder, None] {
            assert_eq!(
                decide(
                    DropTarget::PaneTree {
                        side: Side::Left,
                        folder
                    },
                    &paths
                ),
                DropOutcome::Transfer {
                    side: Side::Left,
                    folder,
                    paths: paths.clone()
                }
            );
        }
    }
    assert_eq!(
        decide(
            DropTarget::PaneTree {
                side: Side::Left,
                folder: None
            },
            &[]
        ),
        DropOutcome::Ignore
    );
    // Home's targets keep W4's rules.
    assert_eq!(
        decide(
            DropTarget::PathField(FieldId::HomeLeft),
            &[a.clone(), b.clone()]
        ),
        DropOutcome::Fill(vec![
            (FieldId::HomeLeft, "/a".into()),
            (FieldId::HomeRight, "/b".into())
        ])
    );
}

#[test]
fn a9_19_decide_rows_follows_the_table() {
    let mut rows = hikasa_folder::select::Selection::default();
    rows.insert(
        hikasa_folder::NodeId(5),
        hikasa_folder::select::SideSet::LEFT,
    );
    let drag = RowDrag {
        from: Some(Side::Left),
        rows: rows.clone(),
    };
    let folder = hikasa_folder::NodeId(7);
    let other = |folder| RowTarget::Pane {
        side: Side::Right,
        folder,
    };
    assert_eq!(
        decide_rows(&drag, other(None), false),
        RowDrop::ToOtherSide {
            from: Side::Left,
            moving: false
        }
    );
    assert_eq!(
        decide_rows(&drag, other(None), true),
        RowDrop::ToOtherSide {
            from: Side::Left,
            moving: true
        }
    );
    assert_eq!(
        decide_rows(&drag, other(Some(folder)), false),
        RowDrop::IntoFolder {
            from: Side::Left,
            folder,
            moving: false
        }
    );
    assert_eq!(
        decide_rows(&drag, other(Some(folder)), true),
        RowDrop::IntoFolder {
            from: Side::Left,
            folder,
            moving: true
        }
    );
    // The own pane, a folder row of it, the centre and everything outside do nothing.
    let own = |folder| RowTarget::Pane {
        side: Side::Left,
        folder,
    };
    assert_eq!(decide_rows(&drag, own(None), false), RowDrop::Nothing);
    assert_eq!(
        decide_rows(&drag, own(Some(folder)), true),
        RowDrop::Nothing
    );
    assert_eq!(
        decide_rows(&drag, RowTarget::Elsewhere, false),
        RowDrop::Nothing
    );
    assert_eq!(
        decide_rows(
            &drag,
            RowTarget::Pane {
                side: Side::Center,
                folder: None
            },
            false
        ),
        RowDrop::Nothing
    );
    // Rows of both sides (pressed in the centre) go to the pane they are released on, from the other side.
    let both = RowDrag { from: None, rows };
    assert_eq!(
        decide_rows(&both, own(None), false),
        RowDrop::ToOtherSide {
            from: Side::Right,
            moving: false
        }
    );
    let empty = RowDrag {
        from: Some(Side::Left),
        rows: hikasa_folder::select::Selection::default(),
    };
    assert_eq!(decide_rows(&empty, other(None), false), RowDrop::Nothing);
}

/// The fixture: `l` and `r` with a folder `d` on both sides, and outside sources in `work/src` (twins in `work/twin`).
fn fixture(world: &World) {
    put(world, "l", "d/x.txt", "x\n", 1_000);
    put(world, "r", "d/x.txt", "x\n", 1_000);
    put(world, "r", "d/exists.txt", "old right\n", 1_000);
    // The twin is the same tree with the same times (Newer compares them).
    let touch = crate::tests_folder_ops::touch;
    for root in ["src", "twin/src"] {
        touch(
            &world.write(&format!("work/{root}/one.txt"), "one\n"),
            2_000,
        );
        touch(
            &world.write(&format!("work/{root}/exists.txt"), "new\n"),
            2_000,
        );
        touch(
            &world.write(&format!("work/{root}/sub/two.txt"), "two\n"),
            2_000,
        );
    }
    touch(&world.write("work/twin/d/x.txt", "x\n"), 1_000);
    touch(&world.write("work/twin/d/exists.txt", "old right\n"), 1_000);
}

fn sources(world: &World, root: &str) -> Vec<PathBuf> {
    ["one.txt", "exists.txt", "sub"]
        .iter()
        .map(|n| world.path(&format!("work/{root}/{n}")))
        .collect()
}

/// `hikasa vfs cp|mv` of the same sources into the twin of `d`.
fn cli(world: &World, moving: bool, overwrite: OverwritePolicy) {
    let svc = &world.service;
    let sources: Vec<OsString> = sources(world, "twin/src")
        .into_iter()
        .map(PathBuf::into_os_string)
        .collect();
    let mut target = world.path("work/twin/d").into_os_string();
    target.push(std::path::MAIN_SEPARATOR_STR);
    let request = if moving {
        VfsOpRequest::Move {
            sources,
            target,
            options: MoveOptions {
                overwrite,
                ..MoveOptions::default()
            },
        }
    } else {
        VfsOpRequest::Copy {
            sources,
            target,
            options: CopyOptions {
                recursive: true,
                overwrite,
                ..CopyOptions::default()
            },
        }
    };
    svc.spawn(JobKind::VfsWrite, "twin".into(), move |svc, job| {
        svc.vfs_run(&request, job)
    })
    .wait()
    .expect("vfs run");
}

fn drop_into_d(world: &World, app: &mut TestApp, view: hikasa_session::ipc::ViewId) {
    let folder = node_of(app, view, "d");
    let w = app.win();
    app.0.drop_paths(
        w,
        DropTarget::PaneTree {
            side: Side::Right,
            folder,
        },
        sources(world, "src"),
    );
    world.settle(app);
}

#[test]
fn a9_19_an_os_drop_copies_the_plan_it_shows() {
    for (moving, overwrite) in [
        (false, OverwritePolicy::Fail),
        (false, OverwritePolicy::Skip),
        (false, OverwritePolicy::Always),
        (true, OverwritePolicy::Always),
        (true, OverwritePolicy::Newer),
    ] {
        let world = World::new();
        fixture(&world);
        let mut app = world.app(None);
        let view = open(&world, &mut app);
        drop_into_d(&world, &mut app, view);
        let d = drop_dialog(&mut app).expect("the prompt");
        assert_eq!(
            d.message(),
            format!(
                "Copy (or move) 3 items into `{}`?",
                world.path("work/r/d").display()
            )
        );
        d.set_overwrite(overwrite);
        d.show_move = moving;
        world.settle(&mut app);
        // The preview is core's dry run of the plan; nothing is written yet.
        let d = drop_dialog(&mut app).expect("the prompt");
        let (plan, lines) = d.shown().expect("planned").clone().expect("a plan");
        assert!(
            lines.last().is_some_and(|l| l.starts_with("would ")),
            "{lines:?}"
        );
        assert!(!listing(&world, "work/r/d").contains_key("one.txt"));
        let shown = serde_json::to_string(&plan).expect("json");
        press(
            &world,
            &mut app,
            if moving {
                ButtonRole::Other(2)
            } else {
                ButtonRole::Other(1)
            },
        );
        assert_eq!(
            folder_model(&mut app, view).executed.as_deref(),
            Some(shown.as_str()),
            "the plan shown is the plan run"
        );
        cli(&world, moving, overwrite);
        assert_eq!(
            listing(&world, "work/r/d"),
            listing(&world, "work/twin/d"),
            "moving {moving}, {overwrite:?}"
        );
        assert_eq!(
            listing(&world, "work/src"),
            listing(&world, "work/twin/src")
        );
        // The folder is refreshed: its new rows show.
        assert!(
            node_of(&mut app, view, "d/one.txt").is_some(),
            "the drop's folder is refreshed"
        );
        assert!(
            notices(&app)
                .iter()
                .any(|(_, text, _)| text.contains("dropped items"))
        );
    }
}

#[test]
fn a9_19_paths_dropped_on_a_path_edit_become_its_base() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let w = app.win();
    app.0.drop_paths(
        w,
        DropTarget::PanePath(Side::Left),
        vec![world.path("work/src"), world.path("work/twin/src")],
    );
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert_eq!(
        [
            m.path_edit(0).committed.clone(),
            m.path_edit(1).committed.clone()
        ],
        [
            world.path("work/src").display().to_string(),
            world.path("work/twin/src").display().to_string()
        ]
    );
}

#[test]
fn a9_19_rows_dragged_are_copy_side_and_copy_to_folder() {
    let world = World::new();
    put(&world, "l", "a.txt", "left a\n", 3_000);
    put(&world, "l", "e/y.txt", "y\n", 1_000);
    put(&world, "r", "a.txt", "right a\n", 1_000);
    put(&world, "r", "f/z.txt", "z\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::ExpandAll);
    select(&mut app, view, "a.txt");
    let w = app.win();
    let rows = folder_model(&mut app, view).selection().items.clone();
    let drag = RowDrag {
        from: Some(Side::Left),
        rows,
    };
    // To the other pane: CopySide of the dragged rows, through D-244's flow (the confirmation shows its preview).
    let other = RowTarget::Pane {
        side: Side::Right,
        folder: None,
    };
    let before = dialogs(&app);
    app.0.drop_rows(w, &drag, other, false);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), before + 1, "CopySide's confirmation");
    let title = crate::tests_folder_ops::top_title(&app);
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    app.0.execute(
        w,
        crate::window::Invocation::with(
            CommandId::CopySide,
            crate::window::CommandArg::Side(Side::Left),
        ),
    );
    world.settle(&mut app);
    assert_eq!(
        crate::tests_folder_ops::top_title(&app),
        title,
        "the same flow"
    );
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    // Shift: MoveSide.
    app.0.drop_rows(w, &drag, other, true);
    world.settle(&mut app);
    let moved = crate::tests_folder_ops::top_title(&app);
    assert!(
        moved.as_deref().is_some_and(|t| t.starts_with("Move")),
        "{moved:?}"
    );
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    // Onto a folder row of the other pane: Copy to Folder, prefilled with that folder, *relative*.
    let folder = node_of(&mut app, view, "f").expect("f");
    app.0.drop_rows(
        w,
        &drag,
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
    match d.form() {
        OpForm::CopyTo {
            moving,
            target,
            structure,
            ..
        } => {
            assert!(!moving);
            assert_eq!(*target, world.path("work/r/f").display().to_string());
            assert_eq!(*structure, PathStructure::Relative);
        }
        _ => panic!("not Copy to Folder"),
    }
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    // The own pane and outside: nothing.
    let before = dialogs(&app);
    for target in [
        RowTarget::Pane {
            side: Side::Left,
            folder: None,
        },
        RowTarget::Elsewhere,
    ] {
        app.0.drop_rows(w, &drag, target, false);
        world.settle(&mut app);
    }
    assert_eq!(dialogs(&app), before);
    // Rows that are no longer the selection do nothing.
    select(&mut app, view, "e");
    app.0.drop_rows(w, &drag, other, false);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), before);
}

#[test]
fn a9_19_nothing_is_dropped_during_an_operation() {
    let world = World::new();
    fixture(&world);
    put(&world, "l", "a.txt", "left a\n", 3_000);
    put(&world, "r", "a.txt", "right a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    // An operation holds the view: its confirmation waits, then its job waits at a gate.
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    let before = dialogs(&app);
    let folder = node_of(&mut app, view, "d");
    let w = app.win();
    let drop = |app: &mut TestApp| {
        app.0.drop_paths(
            w,
            DropTarget::PaneTree {
                side: Side::Right,
                folder,
            },
            sources(&world, "src"),
        );
    };
    drop(&mut app);
    assert_eq!(dialogs(&app), before, "no prompt beside the confirmation");
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(std::sync::Arc::clone(&g));
    let title = crate::tests_folder_ops::top_title(&app);
    app.0.dialog_button(w, ButtonRole::Yes);
    for _ in 0..200 {
        app.0.handle(crate::app::UiMsg::Tick);
        if folder_model(&mut app, view).busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        folder_model(&mut app, view).busy(),
        "the operation runs ({title:?})"
    );
    let before = dialogs(&app);
    drop(&mut app);
    assert_eq!(dialogs(&app), before, "no prompt while it runs");
    assert!(
        notices(&app)
            .iter()
            .any(|(_, text, _)| text == "Wait for the current operation to end."),
        "{:?}",
        notices(&app)
    );
    set_gate(&g, true);
    world.settle(&mut app);
    assert!(!listing(&world, "work/r/d").contains_key("one.txt"));
    // After the operation the drop is asked; while its prompt is open the view takes no second one (the model's own
    // check, below the window's).
    drop(&mut app);
    world.settle(&mut app);
    assert!(drop_dialog(&mut app).is_some(), "the prompt once idle");
    assert!(
        !folder_model(&mut app, view).start_drop_prompt(),
        "the view waits for the first prompt"
    );
}
