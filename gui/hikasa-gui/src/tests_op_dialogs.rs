//! M2 W9b (§1.9.12, §1.9.13; D-244, D-245, D-256): the operation dialogs over core previews, through the models
//! (A9.12 GUI part, A9.13, A9.14). Each verb runs through its dialog on a disk tree, and the same request made through
//! the Service as `hikasa folder op <verb> --path … --yes` makes it (a twin tree) leaves the same tree. Files live in a
//! private temporary folder; no recycle bin, no real application (a recording launcher).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::{ConfirmMode, FolderOp, PathStructure, RenameMask, TouchFrom};
use hikasa_folder::select::SideSet;
use hikasa_session::api::human::folder_op_lines;
use hikasa_session::api::{FolderOpRequest, OpOverrides, OpRequest, SelectRequest};
use hikasa_session::ipc::ViewId;
use hikasa_session::launcher::{Recorded, RecordingLauncher};
use hikasa_session::log::LogKind;
use hikasa_vfs::ops::OverwritePolicy;

use crate::app::{AppCore, Effect};
use crate::commands::CommandId;
use crate::content_dialogs::{CompareContentsDialog, ExcludeDialog, QuickCompareDialog};
use crate::dialog::{ButtonRole, DialogModel, Prompt};
use crate::op_dialogs::{OpDialog, OpForm, PlanConfirmDialog, RenameMode, SideChoice, YES_TO_ALL};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{
    dialogs, gate, logged, notices, open, open_with, poll_until, press, put, select, set_gate,
};
use crate::window::{CommandArg, Invocation, OpenWithItem};

// ---- helpers ------------------------------------------------------------------------------------------------------

/// Every file and folder below `work/<side>`: relative path → content (`/` for a folder).
fn tree(world: &World, side: &str) -> BTreeMap<String, String> {
    let root = world.path(&format!("work/{side}"));
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

/// Both sides' trees.
fn trees(world: &World) -> [BTreeMap<String, String>; 2] {
    [tree(world, "l"), tree(world, "r")]
}

/// The modification time of `work/<side>/<rel>` in whole seconds.
fn mtime(world: &World, side: &str, rel: &str) -> u64 {
    std::fs::metadata(world.path(&format!("work/{side}/{rel}")))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// The fixture of the verb tests: a pair, a newer pair, orphans, a folder.
fn fixture(world: &World) {
    put(world, "l", "a.txt", "left a\n", 3_000);
    put(world, "r", "a.txt", "right a\n", 1_000);
    put(world, "l", "d/x.txt", "x left\n", 2_000);
    put(world, "r", "d/x.txt", "x right!\n", 2_500);
    put(world, "l", "only-left.txt", "l\n", 1_000);
    put(world, "r", "only-right.txt", "r\n", 1_000);
}

/// The verb dialog on the window's stack.
fn op_dialog(app: &mut TestApp) -> &mut OpDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<OpDialog>())
        .expect("a verb dialog")
}

fn find<T: DialogModel>(app: &mut TestApp) -> Option<&mut T> {
    let w = app.win();
    app.0.window_mut(w).and_then(|w| w.dialogs.find_mut::<T>())
}

/// Selects `paths` (each on the sides it has) as Ctrl+clicks do.
fn select_all_of(app: &mut AppCore, view: ViewId, paths: &[&str]) {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let nodes: Vec<_> = {
        let read = model.read();
        paths
            .iter()
            .map(|p| {
                read.preorder()
                    .map(|(id, _)| id)
                    .find(|&id| read.path(id).as_str() == *p)
                    .unwrap_or_else(|| panic!("no row {p}"))
            })
            .collect()
    };
    m.with_selection(|sel, _, present| {
        sel.items = Default::default();
        for node in &nodes {
            sel.items.insert(*node, present(*node));
        }
        sel.current = nodes.first().copied();
        sel.moves += 1;
    });
}

/// The same request through the Service on a twin tree, as `hikasa folder op <verb> --path <rows> --yes` makes it.
fn cli_twin(world: &World, op: OpRequest, paths: &[(&str, SideSet)]) {
    let mut app = world.app(None);
    let view = open(world, &mut app);
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let request = FolderOpRequest {
        op,
        select: SelectRequest {
            paths: paths.iter().map(|(p, s)| ((*p).to_owned(), *s)).collect(),
            ..SelectRequest::default()
        },
        read_only: Default::default(),
        overrides: OpOverrides::default(),
    };
    let service = world.service.clone();
    let outcome = service
        .spawn(JobKind::FolderOp, "twin".into(), move |svc, job| {
            let plan = svc.folder_op_plan(&model, &request, job)?;
            let answered = svc
                .folder_confirmation(&plan, Some(ConfirmMode::YesToAll))
                .finish(plan)
                .map_err(|_| hikasa_session::ApiError::Cancelled)?;
            svc.folder_op_execute(answered, job)
        })
        .wait()
        .expect("the twin ran");
    assert!(outcome.ok(), "{:?}", outcome.dto());
}

/// Presses Start and answers every question with Yes to All.
fn start_and_confirm(world: &World, app: &mut TestApp) {
    press(world, app, ButtonRole::Ok);
    for _ in 0..4 {
        if find::<PlanConfirmDialog>(app).is_some() {
            press(world, app, YES_TO_ALL);
        }
    }
    world.settle(app);
}

/// Waits for a dialog's plan.
fn planned(world: &World, app: &mut TestApp) {
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    }
}

// ---- A9.12: the plan executed is the plan previewed --------------------------------------------------------------

/// A9.12 (D-244): the preview list is `folder op … --dry-run`'s output line for line, and the plan the execute job
/// receives serializes equal to the plan the preview was made of, after `Confirmation::finish` only.
#[test]
fn a9_12_the_executed_plan_is_the_previewed_plan() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[file_operations]\nconfirm_copy = false\nconfirm_overwrite_newer = false\n",
    )]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["a.txt", "only-left.txt"]);
    app.execute(CommandId::CopyTo);
    let target = world.path("work/out").to_string_lossy().into_owned();
    op_dialog(&mut app).set_target(&target);
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    let list = d.list().expect("planned");
    let previewed = list.op_plan().expect("an operation").clone();
    let dry = world.service.folder_op_dry_run(previewed.clone());
    assert_eq!(list.texts(), folder_op_lines(&dry.dto(), dry.verb));
    assert!(!list.is_empty());
    let expected = world
        .service
        .folder_confirmation(&previewed, None)
        .finish(previewed)
        .expect("nothing to ask");
    press(&world, &mut app, ButtonRole::Ok);
    world.settle(&mut app);
    let executed = folder_model(&mut app, view)
        .executed
        .clone()
        .expect("executed");
    assert_eq!(executed, serde_json::to_string(&expected).expect("json"));
    assert_eq!(
        std::fs::read_to_string(world.path("work/out/a.txt"))
            .ok()
            .as_deref(),
        Some("left a\n")
    );
}

// ---- A9.13: one test per row of §1.9.13 -------------------------------------------------------------------------

/// Copy/Move Right and Left, the side prompts, Other Side and Exchange plan at once and run through their questions;
/// each leaves the tree `folder op` leaves.
#[test]
fn a9_13_copy_move_sides_and_exchange_equal_the_cli() {
    let both = SideSet::LEFT | SideSet::RIGHT;
    type Case<'a> = (
        Invocation,
        &'a [&'a str],
        OpRequest,
        Vec<(&'a str, SideSet)>,
    );
    let cases: Vec<Case<'_>> = vec![
        (
            Invocation::new(CommandId::MoveRight),
            &["a.txt", "only-left.txt"],
            OpRequest::Op(FolderOp::Move {
                to: Side::Right,
                overwrite: OverwritePolicy::Always,
            }),
            vec![("a.txt", SideSet::LEFT), ("only-left.txt", SideSet::LEFT)],
        ),
        (
            Invocation::new(CommandId::MoveLeft),
            &["d/x.txt"],
            OpRequest::Op(FolderOp::Move {
                to: Side::Left,
                overwrite: OverwritePolicy::Always,
            }),
            vec![("d/x.txt", SideSet::RIGHT)],
        ),
        (
            Invocation::with(CommandId::CopySide, CommandArg::Side(Side::Right)),
            &["only-right.txt", "a.txt"],
            OpRequest::Op(FolderOp::Copy {
                to: Side::Left,
                overwrite: OverwritePolicy::Always,
            }),
            vec![
                ("only-right.txt", SideSet::RIGHT),
                ("a.txt", SideSet::RIGHT),
            ],
        ),
        (
            Invocation::with(CommandId::MoveSide, CommandArg::Side(Side::Left)),
            &["only-left.txt"],
            OpRequest::Op(FolderOp::Move {
                to: Side::Right,
                overwrite: OverwritePolicy::Always,
            }),
            vec![("only-left.txt", SideSet::LEFT)],
        ),
        (
            Invocation::new(CommandId::MoveBoth),
            &["a.txt", "d/x.txt"],
            OpRequest::Op(FolderOp::Exchange),
            vec![("a.txt", both), ("d/x.txt", both)],
        ),
    ];
    for (invocation, rows, op, paths) in cases {
        let (gui, twin) = (World::new(), World::new());
        fixture(&gui);
        fixture(&twin);
        let mut app = gui.app(None);
        let view = open(&gui, &mut app);
        select_all_of(&mut app, view, rows);
        let w = app.win();
        app.0.execute(w, invocation.clone());
        gui.settle(&mut app);
        let confirm = find::<PlanConfirmDialog>(&mut app).expect("the question");
        // A verb without options: the confirmation is the preview dialog, with the list as its body.
        assert!(
            confirm.list().is_some_and(|l| l.len() > 1),
            "{invocation:?}"
        );
        assert_eq!(confirm.include_hidden(), Some(false));
        press(&gui, &mut app, YES_TO_ALL);
        start_and_confirm(&gui, &mut app);
        cli_twin(&twin, op, &paths);
        assert_eq!(trees(&gui), trees(&twin), "{invocation:?}");
        assert!(
            !logged(&app, LogKind::Summary).is_empty(),
            "{invocation:?}: the log"
        );
        assert!(!notices(&app).is_empty(), "{invocation:?}: a notice");
    }
}

/// `CopyPrompt` asks the side (only writable sides with items to copy are offered), then copies as `CopySide`.
#[test]
fn a9_13_copy_to_side_asks_the_side() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.left = true);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyPrompt);
    let prompt = find::<Prompt>(&mut app).expect("the side question");
    let labels: Vec<&str> = prompt.buttons.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(
        labels,
        ["Right", "Cancel"],
        "the read-only left is not offered"
    );
    press(&world, &mut app, ButtonRole::Other(2));
    world.settle(&mut app);
    assert!(
        find::<PlanConfirmDialog>(&mut app).is_some(),
        "the copy's question"
    );
    start_and_confirm(&world, &mut app);
    assert_eq!(
        tree(&world, "r").get("a.txt").map(String::as_str),
        Some("left a\n")
    );
}

/// Copy to Folder and Move to Folder: target, structure, sides and overwrite; the dialog remembers its choices.
#[test]
fn a9_13_copy_and_move_to_folder_equal_the_cli() {
    for moving in [false, true] {
        let (gui, twin) = (World::new(), World::new());
        fixture(&gui);
        fixture(&twin);
        let mut app = gui.app(None);
        let view = open(&gui, &mut app);
        // One row in a folder: *Keep relative folder structure* would drop `d`, *base* keeps it.
        select_all_of(&mut app, view, &["d/x.txt"]);
        app.execute(if moving {
            CommandId::MoveTo
        } else {
            CommandId::CopyTo
        });
        let target = gui.path("work/out").to_string_lossy().into_owned();
        let d = op_dialog(&mut app);
        d.set_target(&target);
        d.edit(|f| {
            if let OpForm::CopyTo {
                structure, sides, ..
            } = f
            {
                *structure = PathStructure::Base;
                *sides = SideChoice::Left;
            }
        });
        planned(&gui, &mut app);
        assert!(op_dialog(&mut app).list().is_some_and(|l| l.acts()));
        start_and_confirm(&gui, &mut app);
        let target = twin.path("work/out").into_os_string();
        let op = if moving {
            OpRequest::MoveTo {
                sides: SideSet::LEFT,
                target,
                structure: PathStructure::Base,
                overwrite: OverwritePolicy::Fail,
            }
        } else {
            OpRequest::CopyTo {
                sides: SideSet::LEFT,
                target,
                structure: PathStructure::Base,
                overwrite: OverwritePolicy::Fail,
            }
        };
        let both = SideSet::LEFT | SideSet::RIGHT;
        cli_twin(&twin, op, &[("d/x.txt", both)]);
        assert_eq!(trees(&gui), trees(&twin), "moving {moving}");
        assert_eq!(tree(&gui, "out"), tree(&twin, "out"), "moving {moving}");
        assert_eq!(
            tree(&gui, "out").get("d/x.txt").map(String::as_str),
            Some("x left\n")
        );
        // Remembered and read again by the next dialog.
        assert_eq!(
            app.0.state().dialog_text("copy-to-folder", "structure"),
            Some("base")
        );
        select(&mut app, view, "a.txt");
        app.execute(CommandId::CopyTo);
        assert!(matches!(
            op_dialog(&mut app).form(),
            OpForm::CopyTo {
                structure: PathStructure::Base,
                ..
            }
        ));
        press(&gui, &mut app, ButtonRole::Cancel);
    }
}

/// Rename: one row inline (Enter commits, Esc cancels, a collision fails the item), more rows in the dialog with the
/// `base_mask` prefill, or regular expressions.
#[test]
fn a9_13_rename_inline_and_with_masks() {
    let (gui, twin) = (World::new(), World::new());
    for w in [&gui, &twin] {
        fixture(w);
        put(w, "l", "b.txt", "b\n", 1_000);
    }
    let mut app = gui.app(None);
    let view = open(&gui, &mut app);
    // Inline: Esc cancels.
    select(&mut app, view, "only-left.txt");
    app.execute(CommandId::Rename);
    let m = folder_model(&mut app, view);
    assert_eq!(
        m.inline_rename().map(|e| e.text.as_str()),
        Some("only-left.txt")
    );
    m.cancel_inline_rename();
    assert!(m.inline_rename().is_none());
    // Enter commits a new name.
    app.execute(CommandId::Rename);
    folder_model(&mut app, view).set_inline_text("renamed.txt");
    let w = app.win();
    app.0.commit_inline_rename(w);
    gui.settle(&mut app);
    start_and_confirm(&gui, &mut app);
    assert!(tree(&gui, "l").contains_key("renamed.txt"));
    // A collision: the item fails, nothing is overwritten.
    select(&mut app, view, "b.txt");
    app.execute(CommandId::Rename);
    folder_model(&mut app, view).set_inline_text("renamed.txt");
    app.0.commit_inline_rename(w);
    gui.settle(&mut app);
    start_and_confirm(&gui, &mut app);
    assert_eq!(
        tree(&gui, "l").get("renamed.txt").map(String::as_str),
        Some("l\n")
    );
    assert_eq!(
        tree(&gui, "l").get("b.txt").map(String::as_str),
        Some("b\n")
    );
    // More rows: the dialog, prefilled with `base_mask` of the names.
    select_all_of(&mut app, view, &["a.txt", "b.txt"]);
    app.execute(CommandId::Rename);
    let d = op_dialog(&mut app);
    let OpForm::Rename { dos, mode, .. } = d.form().clone() else {
        panic!("the Rename form");
    };
    assert_eq!(mode, RenameMode::Dos);
    assert_eq!(
        dos,
        hikasa_folder::ops::base_mask(&["a.txt", "b.txt"]),
        "base_mask's prefill"
    );
    d.edit(|f| {
        if let OpForm::Rename {
            mode,
            old,
            new,
            sides,
            ..
        } = f
        {
            *mode = RenameMode::Regex;
            *old = "(.*)\\.txt".into();
            *new = "$1.bak".into();
            *sides = SideChoice::Left;
        }
    });
    planned(&gui, &mut app);
    start_and_confirm(&gui, &mut app);
    // The twin: the inline rename as `--name`, then the regex.
    cli_twin(
        &twin,
        OpRequest::Op(FolderOp::Rename {
            sides: SideSet::LEFT,
            mask: RenameMask::Name("renamed.txt".into()),
        }),
        &[("only-left.txt", SideSet::LEFT)],
    );
    cli_twin(
        &twin,
        OpRequest::Op(FolderOp::Rename {
            sides: SideSet::LEFT,
            mask: RenameMask::Regex {
                old: "(.*)\\.txt".into(),
                new: "$1.bak".into(),
            },
        }),
        &[("a.txt", SideSet::LEFT), ("b.txt", SideSet::LEFT)],
    );
    assert_eq!(trees(&gui), trees(&twin));
    assert_eq!(
        app.0.state().dialog_text("rename", "old"),
        Some("(.*)\\.txt")
    );
}

/// Touch: a local date and time (parsed by core in the clock's zone) or the other side's times; the last time set is
/// remembered.
#[test]
fn a9_13_touch_sets_the_local_time_or_copies_the_other_side() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Touch);
    let d = op_dialog(&mut app);
    // A bad text replaces the preview with core's message and keeps Start off.
    d.edit(|f| {
        if let OpForm::Touch {
            from_other, text, ..
        } = f
        {
            *from_other = false;
            *text = "not a time".into();
        }
    });
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    assert!(
        d.message().is_some_and(|m| m.contains("YYYY-MM-DD")),
        "{:?}",
        d.message()
    );
    assert!(
        !d.buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && b.enabled)
    );
    let zone = world.service.clock().now().time_zone().clone();
    let at = hikasa_session::api::parse_touch_time("2001-02-03 04:05:06", &zone).expect("a time");
    d.edit(|f| {
        if let OpForm::Touch { text, sides, .. } = f {
            *text = "2001-02-03 04:05:06".into();
            *sides = SideChoice::Both;
        }
    });
    planned(&world, &mut app);
    start_and_confirm(&world, &mut app);
    for side in ["l", "r"] {
        assert_eq!(
            mtime(&world, side, "a.txt"),
            at.unix_seconds() as u64,
            "{side}"
        );
    }
    assert_eq!(
        app.0.state().dialog_text("touch", "last"),
        Some("2001-02-03 04:05:06")
    );
    // Copy timestamps from the other side: the right's d/x.txt takes the left's time.
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    select(&mut app, view, "d/x.txt");
    app.execute(CommandId::Touch);
    op_dialog(&mut app).edit(|f| {
        if let OpForm::Touch { from_other, to, .. } = f {
            *from_other = true;
            *to = Side::Right;
        }
    });
    planned(&world, &mut app);
    start_and_confirm(&world, &mut app);
    // The right side takes the left's time (2,000 s), the left keeps it: the direction is pinned (R25).
    assert_eq!(mtime(&world, "r", "d/x.txt"), 2_000);
    assert_eq!(mtime(&world, "l", "d/x.txt"), 2_000);
    let _ = TouchFrom::OtherSide { to: Side::Right };
}

/// New Folder: in the current row's folder, on the focus side; the sides are remembered.
#[test]
fn a9_13_new_folder_in_the_current_folder() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    select(&mut app, view, "d/x.txt");
    folder_model(&mut app, view)
        .with_selection(|sel, _, _| sel.focus = SideSet::LEFT | SideSet::RIGHT);
    app.execute(CommandId::NewFolder);
    let d = op_dialog(&mut app);
    let OpForm::NewFolder { parent, sides, .. } = d.form().clone() else {
        panic!("New Folder");
    };
    assert_eq!(parent, "d", "a file's parent");
    assert_eq!(sides, SideChoice::Both, "both sides focused");
    d.edit(|f| {
        if let OpForm::NewFolder { name, sides, .. } = f {
            *name = "made".into();
            *sides = SideChoice::Right;
        }
    });
    planned(&world, &mut app);
    start_and_confirm(&world, &mut app);
    assert_eq!(
        tree(&world, "r").get("d/made").map(String::as_str),
        Some("/")
    );
    assert!(!tree(&world, "l").contains_key("d/made"));
    assert_eq!(
        app.0.state().dialog_text("new-folder", "sides"),
        Some("right")
    );
}

/// Option changes plan again and cancel the older job; *Include hidden items* re-plans; a read-only side's choices are
/// disabled and the Service refuses them again.
#[test]
fn a9_13_option_changes_replan_and_read_only_sides_are_off() {
    let world = World::new();
    fixture(&world);
    put(&world, "l", ".hidden", "h\n", 1_000);
    let mut app = world.app(None);
    let view = open_with(&world, &mut app, |o| o.read_only.right = true);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    select_all_of(&mut app, view, &["a.txt", "only-left.txt"]);
    app.execute(CommandId::Touch);
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    let first = d.plans;
    assert!(!d.side_enabled(SideChoice::Right), "the read-only right");
    assert!(!d.side_enabled(SideChoice::Both));
    assert!(d.side_enabled(SideChoice::Left));
    assert_eq!(
        OpDialog::read_only_tip(Side::Right),
        "Disable editing is on for the right side"
    );
    // Two changes before a poll: one job runs for the last of them.
    d.edit(|f| {
        if let OpForm::Touch { text, .. } = f {
            *text = "2002-01-01 00:00:00".into();
        }
    });
    d.set_include_hidden(true);
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    assert_eq!(d.plans, first + 1, "one plan for both changes");
    assert!(d.include_hidden());
    // Asked to write the right anyway, the Service refuses (D-184).
    d.edit(|f| {
        if let OpForm::Touch { sides, .. } = f {
            *sides = SideChoice::Right;
        }
    });
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    assert!(
        d.message().is_some_and(|m| m.contains("right")),
        "{:?}",
        d.message()
    );
    press(&world, &mut app, ButtonRole::Cancel);
    assert_eq!(dialogs(&app), 0);
}

/// A plan whose comparison changed before Start is not run: the dialog offers *Plan again*, which runs the command
/// once more.
#[test]
fn a9_13_a_changed_comparison_offers_plan_again() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Touch);
    planned(&world, &mut app);
    // The model changes under the dialog (a refresh of the rows).
    let service = world.service.clone();
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    let root = hikasa_folder::NodeId::ROOT;
    service
        .spawn(JobKind::FolderRefresh, "refresh".into(), move |svc, job| {
            svc.folder_refresh(&model, &[root], job, &hikasa_folder::NullObserver)
        })
        .wait()
        .expect("refresh");
    planned(&world, &mut app);
    let d = op_dialog(&mut app);
    assert!(
        d.message().is_some_and(|m| m.contains("Plan again")),
        "{:?}",
        d.message()
    );
    assert!(d.buttons().iter().any(|b| b.label == "Plan again"));
    press(&world, &mut app, crate::op_dialogs::PLAN_AGAIN);
    world.settle(&mut app);
    assert!(
        find::<OpDialog>(&mut app).is_some(),
        "the command ran again"
    );
}

// ---- Ignored, Exclude, Compare Contents, Quick Compare, Compare To, Open With ------------------------------------

/// Ignored updates the view's rules without a re-run and makes the session dirty; the check follows the row.
#[test]
fn a9_13_ignored_updates_the_rules_without_a_rerun() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    assert_eq!(app.checked(CommandId::Ignored), Some(false));
    app.execute(CommandId::Ignored);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(
        Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "no re-run"
    );
    assert_eq!(m.ignored_list(), Some(&["a.txt".to_owned()][..]));
    assert_eq!(app.checked(CommandId::Ignored), Some(true));
    assert!(
        app.tabs()[app.active()]
            .session
            .as_ref()
            .is_some_and(|s| s.dirty)
    );
    app.execute(CommandId::Ignored);
    assert_eq!(app.checked(CommandId::Ignored), Some(false));
}

/// Exclude shows the masks `folder op exclude` computes and, on OK, the view runs again with them.
#[test]
fn a9_13_exclude_reruns_with_the_masks_of_the_cli() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Exclude);
    world.settle(&mut app);
    let d = find::<ExcludeDialog>(&mut app).expect("Exclude");
    d.set_what(hikasa_folder::select::ExcludeWhat::Type);
    world.settle(&mut app);
    let d = find::<ExcludeDialog>(&mut app).expect("Exclude");
    let shown = d.masks().cloned().expect("read").expect("masks");
    let (model, select_request) = folder_model(&mut app, view)
        .selection_request(false)
        .expect("a selection");
    let cli = world
        .service
        .folder_exclude_masks(
            &model,
            &select_request,
            hikasa_folder::select::ExcludeWhat::Type,
        )
        .expect("masks");
    assert_eq!(shown, cli);
    press(&world, &mut app, ButtonRole::Ok);
    for _ in 0..4 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
    }
    let m = folder_model(&mut app, view);
    assert!(
        !Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "re-run"
    );
    let shown: Vec<String> = crate::tests_folder_ops::shown(&mut app, view)
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    assert!(!shown.iter().any(|p| p.ends_with(".txt")), "{shown:?}");
    assert_eq!(app.0.state().dialog_text("exclude", "what"), Some("type"));
}

/// Compare Contents asks first with `confirm_content_compare`, compares, and its results dialog lists the counts.
#[test]
fn a9_13_compare_contents_asks_and_shows_the_results() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[file_operations]\nconfirm_content_compare = true\n",
    )]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["a.txt", "d"]);
    app.execute(CommandId::Compare);
    world.settle(&mut app);
    let d = find::<CompareContentsDialog>(&mut app).expect("Compare Contents");
    let count = *d.count().expect("counted").as_ref().expect("pairs");
    assert_eq!(count.pairs, 2, "a.txt and d/x.txt");
    d.method = hikasa_core::ContentMethod::Binary;
    press(&world, &mut app, ButtonRole::Ok);
    let prompt = find::<Prompt>(&mut app).expect("the question");
    assert!(
        prompt.message.contains("2 file pairs"),
        "{}",
        prompt.message
    );
    press(&world, &mut app, ButtonRole::Yes);
    world.settle(&mut app);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let results = find::<crate::content_dialogs::ContentsResultsDialog>(&mut app).expect("results");
    assert_eq!(results.len(), 2);
    assert!(!results.is_empty());
    assert!(
        results.counts().iter().any(|l| l.contains(": 2")),
        "{:?}",
        results.counts()
    );
    assert_eq!(
        app.0.state().dialog_text("compare-contents", "method"),
        Some("binary")
    );
}

/// D-256 for Compare Contents (review F3): its end beeps after ≥ 10 s with `beep_after_long_operations`, not before
/// (the duration injected into the view, the path from the job's end to the window's effect real).
#[test]
fn compare_contents_beeps_after_a_long_run() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tweaks.file_operations]\nbeep_after_long_operations = true\n",
    )]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    for (secs, beeps) in [(10, true), (9, false)] {
        select_all_of(&mut app, view, &["a.txt"]);
        folder_model(&mut app, view).fake_elapsed =
            Some(Duration::from_millis(secs * 1_000 - u64::from(!beeps)));
        app.execute(CommandId::Compare);
        world.settle(&mut app);
        let d = find::<CompareContentsDialog>(&mut app).expect("Compare Contents");
        d.show_results = false;
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Ok);
        let effects = world.settle(&mut app);
        assert!(
            logged(&app, LogKind::Summary)
                .iter()
                .any(|l| l.starts_with("Compare Contents")),
            "{:?}",
            logged(&app, LogKind::Summary)
        );
        assert_eq!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Beep(x) if *x == w)),
            beeps,
            "{secs} s: {effects:?}"
        );
    }
}

/// Quick Compare's results equal `hikasa quick`'s for each pair; Open opens the selected one.
#[test]
fn a9_13_quick_compare_equals_quick_and_opens_a_pair() {
    let world = World::new();
    fixture(&world);
    put(&world, "l", "same.txt", "same\n", 1_000);
    put(&world, "r", "same.txt", "same\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["a.txt", "same.txt", "only-left.txt"]);
    app.execute(CommandId::QuickCompare);
    world.settle(&mut app);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let d = find::<QuickCompareDialog>(&mut app).expect("Quick Compare");
    assert_eq!(d.rows().len(), 2, "the orphan is no pair");
    for row in d.rows().to_vec() {
        let request = hikasa_session::api::QuickRequest {
            left: world.path(&format!("work/l/{}", row.path)).into_os_string(),
            right: world.path(&format!("work/r/{}", row.path)).into_os_string(),
            method: d.method(),
            ..Default::default()
        };
        let quick = world
            .service
            .spawn(JobKind::QuickCompare, "quick".into(), move |svc, job| {
                svc.quick(&request, job)
            })
            .wait()
            .expect("quick");
        assert_eq!(
            row.result.clone().ok(),
            Some(quick.outcome.result.kind()),
            "{}",
            row.path
        );
    }
    let d = find::<QuickCompareDialog>(&mut app).expect("Quick Compare");
    d.selected = Some(0);
    let tabs = app.tabs().len();
    press(&world, &mut app, crate::content_dialogs::OPEN);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 1, "the pair opened");
}

/// Compare To: two items, one and the mark, or one and an item from the browse dialog.
#[test]
fn a9_13_compare_to_two_items_the_mark_or_a_browsed_one() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    // Two items (two orphans on different sides).
    select_all_of(&mut app, view, &["only-left.txt", "only-right.txt"]);
    let tabs = app.tabs().len();
    app.execute(CommandId::CompareTo);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 1);
    app.activate(app.tabs().iter().position(|t| t.id == view).expect("view"));
    // One item and the mark.
    select(&mut app, view, "only-right.txt");
    folder_model(&mut app, view).space();
    select(&mut app, view, "only-left.txt");
    app.execute(CommandId::CompareTo);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 2);
    app.activate(app.tabs().iter().position(|t| t.id == view).expect("view"));
    folder_model(&mut app, view).unmark();
    // One item: the browse dialog asks for the second (file mode for a file).
    select(&mut app, view, "only-left.txt");
    app.execute(CommandId::CompareTo);
    let browse = find::<crate::browse::BrowseDialog>(&mut app).expect("browse");
    assert!(matches!(browse.mode, crate::browse::BrowseMode::File));
    let w = app.win();
    let second = world.path("work/r/a.txt").to_string_lossy().into_owned();
    app.0.dialog_button(w, ButtonRole::Cancel);
    app.0.browsed(
        w,
        crate::browse::BrowseTarget::CompareTo(view),
        Some((second, false)),
        false,
    );
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 3);
}

/// Open With ▸: a view of a kind that exists, an application through the launcher (`refresh_when_finished` refreshes
/// the rows), the associated application.
#[test]
fn a9_13_open_with_views_applications_and_the_associated_one() {
    let launcher = Arc::new(RecordingLauncher::default());
    let recording = Arc::clone(&launcher);
    let world = World::with_init(move |init| init.launcher = recording);
    std::fs::create_dir_all(world.path("config")).expect("mkdir");
    std::fs::write(
        world.path("config/openwith.toml"),
        "schema_version = 1\n[[app]]\ndescription = \"Viewer\"\ncommand = \"viewer %F\"\nrefresh_when_finished = true\n",
    )
    .expect("write");
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    assert_eq!(app.0.open_with_names(), ["Viewer"]);
    let w = app.win();
    let tabs = app.tabs().len();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::TextCompare),
        ),
    );
    world.settle(&mut app);
    assert_eq!(
        app.tabs().len(),
        tabs + 1,
        "a Text Compare view of the pair"
    );
    app.activate(app.tabs().iter().position(|t| t.id == view).expect("view"));
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::App(0)),
        ),
    );
    world.settle(&mut app);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let runs: Vec<Recorded> = launcher.calls();
    assert!(
        runs.iter()
            .any(|r| matches!(r, Recorded::Run(c) if c.program == "viewer")),
        "{runs:?}"
    );
    app.execute(CommandId::AssocApp);
    world.settle(&mut app);
    assert!(
        launcher
            .calls()
            .iter()
            .any(|r| matches!(r, Recorded::Associated(p) if p.ends_with("a.txt"))),
        "{:?}",
        launcher.calls()
    );
}

// ---- After the W9b review: the review's surviving mutations (R01, R02, R03, R04, R06, R10, R11, R12, R22) ----------

/// R01: an option changed since the shown plan turns Start off until the new plan: Start then does nothing.
#[test]
fn start_waits_for_the_plan_of_the_changed_options() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["only-left.txt"]);
    app.execute(CommandId::CopyTo);
    op_dialog(&mut app).set_target(&world.path("work/out").to_string_lossy());
    planned(&world, &mut app);
    assert!(
        op_dialog(&mut app)
            .buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && b.enabled)
    );
    op_dialog(&mut app).set_target(&world.path("work/other").to_string_lossy());
    assert!(
        !op_dialog(&mut app)
            .buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && b.enabled),
        "Start is off while the plan of the new target is not made"
    );
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    assert!(find::<OpDialog>(&mut app).is_some(), "nothing started");
    assert!(op_dialog(&mut app).take_started().is_none());
    planned(&world, &mut app);
    start_and_confirm(&world, &mut app);
    assert!(
        world.path("work/other/only-left.txt").is_file(),
        "the new target's plan ran"
    );
    assert!(!world.path("work/out").exists());
}

/// R02, R22: Start pressed after the comparison changed and before the dialog noticed (no poll in between) runs
/// nothing: the dialog turns to *Plan again* (R22); a plan handed over all the same is refused by the view (R02).
#[test]
fn a_changed_comparison_is_refused_by_the_dialog_and_by_the_view() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = trees(&world);
    select_all_of(&mut app, view, &["only-left.txt"]);
    app.execute(CommandId::CopyTo);
    op_dialog(&mut app).set_target(&world.path("work/out").to_string_lossy());
    planned(&world, &mut app);
    let refresh = |world: &World, app: &mut TestApp| {
        let model = Arc::clone(folder_model(app, view).prepared().expect("p").model());
        let root = hikasa_folder::NodeId::ROOT;
        world
            .service
            .spawn(JobKind::FolderRefresh, "refresh".into(), move |svc, job| {
                svc.folder_refresh(&model, &[root], job, &hikasa_folder::NullObserver)
            })
            .wait()
            .expect("refresh");
    };
    refresh(&world, &mut app);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    let d = op_dialog(&mut app);
    assert!(
        d.message().is_some_and(|m| m.contains("Plan again")),
        "{:?}",
        d.message()
    );
    assert!(d.take_started().is_none());
    press(&world, &mut app, ButtonRole::Cancel);
    // The view's own check: a plan handed over after the comparison changed is refused with a warning (the dialog's
    // Start called here, so the window does not hand the plan over at once).
    world.settle(&mut app);
    select_all_of(&mut app, view, &["only-left.txt"]);
    app.execute(CommandId::CopyTo);
    op_dialog(&mut app).set_target(&world.path("work/out").to_string_lossy());
    planned(&world, &mut app);
    let service = world.service.clone();
    let step = op_dialog(&mut app).on_button(ButtonRole::Ok, &service);
    assert!(matches!(step, crate::dialog::DialogStep::Close));
    let started = op_dialog(&mut app).take_started();
    assert!(started.is_some(), "Start handed the plan over");
    refresh(&world, &mut app);
    folder_model(&mut app, view).op_dialog_closed(&service, view, started);
    world.settle(&mut app);
    assert!(
        notices(&app).iter().any(|(_, t, _)| t.contains("changed")),
        "{:?}",
        notices(&app)
    );
    assert_eq!(trees(&world), before);
    assert!(!world.path("work/out").exists(), "nothing was written");
}

/// R03: the selection changed while a verb dialog was open: Start writes nothing and says why.
#[test]
fn a_selection_moved_under_a_verb_dialog_writes_nothing() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["only-left.txt"]);
    app.execute(CommandId::CopyTo);
    op_dialog(&mut app).set_target(&world.path("work/out").to_string_lossy());
    planned(&world, &mut app);
    select_all_of(&mut app, view, &["a.txt"]);
    start_and_confirm(&world, &mut app);
    assert!(
        notices(&app)
            .iter()
            .any(|(_, t, _)| t.contains("selection changed")),
        "{:?}",
        notices(&app)
    );
    assert!(!world.path("work/out").exists(), "nothing was written");
}

/// R04: …to Side… offers only a side that can take the items: a move needs its source writable too.
#[test]
fn the_side_prompt_offers_only_writable_moves() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let mut policy = crate::folder_view::WritePolicy::default();
    policy.read_only.left = true;
    folder_model(&mut app, view).set_write_policy(policy);
    select_all_of(&mut app, view, &["a.txt"]);
    let labels = |app: &mut TestApp| -> Option<Vec<String>> {
        find::<Prompt>(app).map(|p| p.buttons.iter().map(|(_, l)| l.clone()).collect())
    };
    // Copy: only to the right (the left is read-only).
    app.execute(CommandId::CopyPrompt);
    assert_eq!(
        labels(&mut app),
        Some(vec!["Right".to_owned(), "Cancel".to_owned()])
    );
    press(&world, &mut app, ButtonRole::Cancel);
    // Move: to the right would delete on the read-only left: nothing to offer.
    app.execute(CommandId::MovePrompt);
    assert_eq!(labels(&mut app), None, "no side can take a move");
}

/// R06: the inline Rename committed with the name unchanged does nothing: no plan, no question, no entry.
#[test]
fn an_unchanged_inline_name_does_nothing() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Rename);
    let entries = app.0.log().entries().count();
    let w = app.win();
    app.0.commit_inline_rename(w);
    assert!(app.jobs(w).is_empty(), "no plan job");
    world.settle(&mut app);
    assert!(find::<PlanConfirmDialog>(&mut app).is_none());
    assert_eq!(app.0.log().entries().count(), entries, "nothing logged");
    assert!(notices(&app).is_empty(), "{:?}", notices(&app));
}

/// R10: Synchronize ▸ takes the `folder-sync` defaults' recycle bin (and its fallback).
#[test]
fn synchronize_takes_the_folder_sync_defaults_recycle_bin() {
    let world = World::new();
    fixture(&world);
    let mut defaults = world
        .service
        .session_defaults(hikasa_session::sessions::SessionType::FolderSync)
        .expect("defaults");
    let hikasa_session::sessions::SessionRules::Sync(rules) = &mut defaults.rules else {
        panic!("folder-sync rules");
    };
    let factory = rules.sync.recycle;
    rules.sync.recycle = !factory;
    rules.sync.recycle_fallback = true;
    world
        .service
        .session_save_defaults(&defaults)
        .expect("save");
    let mut app = world.app(None);
    let _view = open(&world, &mut app);
    app.execute(CommandId::MirrorRight);
    world.settle(&mut app);
    let d = find::<crate::folder_view::sync::SyncDialog>(&mut app).expect("Synchronize");
    assert_eq!(d.options.recycle, !factory, "the defaults' bin");
    assert!(d.options.recycle_fallback);
}

/// R11: Exclude adds each mask once, whatever the rules already hold.
#[test]
fn exclude_adds_each_mask_once() {
    let mut list = vec!["*.log".to_owned()];
    let new = ["*.log", "a.txt", "a.txt", "b\\"].map(str::to_owned);
    crate::app::ops::add_masks(&mut list, &new);
    assert_eq!(list, ["*.log", "a.txt", "b\\"]);
}

/// R12: a verb dialog opens with the sides the selection has items on: both, or the one side.
#[test]
fn a_verb_dialog_presets_the_sides_of_the_selection() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    for (paths, want) in [
        (&["a.txt"][..], SideChoice::Both),
        (&["only-left.txt"][..], SideChoice::Left),
        (&["only-right.txt"][..], SideChoice::Right),
    ] {
        select_all_of(&mut app, view, paths);
        app.execute(CommandId::CopyTo);
        let OpForm::CopyTo { sides, .. } = op_dialog(&mut app).form().clone() else {
            panic!("Copy to Folder");
        };
        assert_eq!(sides, want, "{paths:?}");
        press(&world, &mut app, ButtonRole::Cancel);
    }
}

// ---- After the W9b review: Copy to Folder's relative target (F13) -----------------------------------------------------

/// A relative target of Copy to Folder is the source side's base folder's (the focus side's, else the side with the
/// items), never the program's current folder; the dialog shows it absolute before Start, and the recent folders keep
/// it absolute.
#[test]
fn a_relative_copy_to_target_is_the_source_base_s() {
    for (side, base) in [(SideSet::RIGHT, "r"), (SideSet::LEFT, "l")] {
        let world = World::new();
        fixture(&world);
        let mut app = world.app(None);
        let view = open(&world, &mut app);
        select_all_of(&mut app, view, &["a.txt"]);
        folder_model(&mut app, view).with_selection(|sel, _, _| sel.focus = side);
        app.execute(CommandId::CopyTo);
        let d = op_dialog(&mut app);
        d.set_target("out");
        d.edit(|f| {
            if let OpForm::CopyTo { sides, .. } = f {
                *sides = if side == SideSet::RIGHT {
                    SideChoice::Right
                } else {
                    SideChoice::Left
                };
            }
        });
        let expected = world.path(&format!("work/{base}/out"));
        let resolved = op_dialog(&mut app)
            .resolved_target()
            .expect("a target")
            .expect("resolved");
        assert_eq!(
            hikasa_session::api::spec_os_string(&resolved),
            expected.clone().into_os_string(),
            "shown absolute before Start"
        );
        planned(&world, &mut app);
        start_and_confirm(&world, &mut app);
        assert!(
            expected.join("a.txt").is_file(),
            "{side:?}: copied into {expected:?}"
        );
        assert_eq!(
            app.0
                .state()
                .mru(hikasa_session::settings::state::MruList::Folders)
                .first(),
            Some(&expected.to_string_lossy().into_owned()),
            "the recent folder is absolute"
        );
    }
}

// ---- After the W9b review: launches (F5, F10, R14, R15) -------------------------------------------------------------

/// A launcher that records its calls (a [`RecordingLauncher`]) and holds each at a gate first.
struct GatedLauncher {
    inner: RecordingLauncher,
    gate: crate::folder_view::Gate,
    /// Calls that reached the gate.
    arrived: std::sync::atomic::AtomicUsize,
}

impl GatedLauncher {
    fn new(gate: &crate::folder_view::Gate) -> GatedLauncher {
        GatedLauncher {
            inner: RecordingLauncher::default(),
            gate: Arc::clone(gate),
            arrived: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn wait(&self) {
        self.arrived
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (open, turn) = &*self.gate;
        let mut open = open.lock().expect("gate");
        while !*open {
            open = turn.wait(open).expect("gate");
        }
    }
}

impl hikasa_session::launcher::AppLauncher for GatedLauncher {
    fn run(
        &self,
        command: &hikasa_session::launcher::OpenCommand,
    ) -> Result<i32, hikasa_session::ApiError> {
        self.wait();
        self.inner.run(command)
    }

    fn open_associated(&self, path: &std::path::Path) -> Result<(), hikasa_session::ApiError> {
        self.wait();
        self.inner.open_associated(path)
    }
}

const VIEWER: &str = "schema_version = 1\n[[app]]\ndescription = \"Viewer\"\ncommand = \"viewer %F\"\nrefresh_when_finished = true\n";

fn world_with(launcher: Arc<dyn hikasa_session::launcher::AppLauncher>) -> World {
    let world = World::with_init(move |init| init.launcher = launcher);
    std::fs::create_dir_all(world.path("config")).expect("mkdir");
    std::fs::write(world.path("config/openwith.toml"), VIEWER).expect("write");
    world
}

fn associated(calls: &[Recorded]) -> Vec<String> {
    calls
        .iter()
        .filter_map(|c| match c {
            Recorded::Associated(p) => Some(p.to_string_lossy().replace('\\', "/")),
            Recorded::Run(_) => None,
        })
        .collect()
}

/// `refresh_when_finished` refreshes the rows' folders by path when the application ends, in the model as it is then:
/// a re-run while the application ran renumbered the rows (review F10, R15).
#[test]
fn refresh_when_finished_refreshes_the_folders_by_path() {
    let g = gate(false);
    let launcher = Arc::new(GatedLauncher::new(&g));
    let world = world_with(launcher.clone());
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["d/x.txt"]);
    let d_before = crate::tests_folder_ops::node_of(&mut app, view, "d").expect("d");
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::App(0)),
        ),
    );
    // While the application runs, the comparison runs again with more rows before `d`: the ids move.
    for i in 0..5 {
        put(&world, "l", &format!("c/{i}.txt"), "c\n", 1_000);
    }
    app.execute(CommandId::FullRefresh);
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        !m.scanning() && m.prepared().is_some_and(|p| p.model().read().len() >= 12)
    });
    let d_after = crate::tests_folder_ops::node_of(&mut app, view, "d").expect("d");
    assert_ne!(d_before, d_after, "the re-run moved the ids");
    put(&world, "l", "d/new.txt", "new\n", 1_000);
    assert!(crate::tests_folder_ops::node_of(&mut app, view, "d/new.txt").is_none());
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        crate::tests_folder_ops::node_of(a, view, "d/new.txt").is_some()
    });
    assert_eq!(launcher.inner.calls().len(), 2, "d/x.txt on both sides");
}

/// The associated application opens every selected item, folders as files, in tree order (R14); more than ten items
/// ask first with the count, No starts nothing (review F5).
#[test]
fn assoc_app_opens_every_item_and_asks_above_ten() {
    let launcher = Arc::new(RecordingLauncher::default());
    let world = world_with(launcher.clone());
    fixture(&world);
    for i in 0..9 {
        put(&world, "l", &format!("many/f{i}.txt"), "m\n", 1_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(&mut app, view, &["a.txt", "d", "only-left.txt"]);
    app.execute(CommandId::AssocApp);
    world.settle(&mut app);
    let got = associated(&launcher.calls());
    assert_eq!(
        got.len(),
        5,
        "a.txt and d on both sides, only-left.txt: {got:?}"
    );
    for end in ["l/a.txt", "r/a.txt", "l/d", "r/d", "l/only-left.txt"] {
        assert!(got.iter().any(|p| p.ends_with(end)), "{end}: {got:?}");
    }
    // Eleven items: the question names the count; No starts nothing, Yes starts all.
    let mut paths: Vec<String> = (0..9).map(|i| format!("many/f{i}.txt")).collect();
    paths.extend(["a.txt".to_owned()]);
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    select_all_of(&mut app, view, &refs);
    for (role, more) in [(ButtonRole::Cancel, 0), (ButtonRole::Yes, 11)] {
        let before = associated(&launcher.calls()).len();
        app.execute(CommandId::AssocApp);
        world.settle(&mut app);
        let prompt = find::<Prompt>(&mut app).expect("the question");
        assert!(
            prompt.message.contains("Open 11 items"),
            "{}",
            prompt.message
        );
        press(&world, &mut app, role);
        world.settle(&mut app);
        assert_eq!(
            associated(&launcher.calls()).len(),
            before + more,
            "{role:?}"
        );
    }
    // Open With an application asks the same way.
    let before = launcher.calls().len();
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::App(0)),
        ),
    );
    world.settle(&mut app);
    let prompt = find::<Prompt>(&mut app).expect("the question");
    assert!(
        prompt.message.contains("Open 11 items with Viewer"),
        "{}",
        prompt.message
    );
    press(&world, &mut app, ButtonRole::Cancel);
    world.settle(&mut app);
    assert_eq!(launcher.calls().len(), before);
}

/// The first item that cannot start stops the rest, with an error notice that names it (review F5).
#[test]
fn the_first_launch_failure_stops_the_rest() {
    let launcher = Arc::new(RecordingLauncher::with_answers(vec![
        Some(0),
        Some(0),
        None,
    ]));
    let world = world_with(launcher.clone());
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(
        &mut app,
        view,
        &["a.txt", "only-left.txt", "only-right.txt"],
    );
    app.execute(CommandId::AssocApp);
    world.settle(&mut app);
    let got = associated(&launcher.calls());
    assert_eq!(
        got.len(),
        3,
        "a.txt's two sides, then only-left.txt fails: {got:?}"
    );
    let errors: Vec<String> = notices(&app)
        .into_iter()
        .filter(|(k, _, _)| *k == crate::notify::NoticeKind::Error)
        .map(|(_, t, _)| t)
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("only-left.txt")
            && errors[0].contains("the item after it was not opened"),
        "{}",
        errors[0]
    );
}

/// Stop (the job indicator's row) between two launches starts nothing more (review F5).
#[test]
fn a_stopped_launch_starts_nothing_more() {
    let g = gate(false);
    let launcher = Arc::new(GatedLauncher::new(&g));
    let world = world_with(launcher.clone());
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select_all_of(
        &mut app,
        view,
        &["only-left.txt", "only-right.txt", "a.txt"],
    );
    app.execute(CommandId::AssocApp);
    let w = app.win();
    assert!(
        app.jobs(w).iter().any(|j| j.view == view),
        "the launch is a job of the view"
    );
    poll_until(&world, &mut app, |_| {
        launcher.arrived.load(std::sync::atomic::Ordering::SeqCst) == 1
    });
    app.0.cancel_jobs(view);
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| a.jobs(w).is_empty());
    assert_eq!(
        associated(&launcher.inner.calls()).len(),
        1,
        "the one held at the gate"
    );
    assert!(
        !notices(&app)
            .iter()
            .any(|(k, _, _)| *k == crate::notify::NoticeKind::Error),
        "a stop is no error"
    );
}

// ---- A9.30 after the W9b review (F9): names that are not valid Unicode ----------------------------------------------

/// `stem`, a code unit that is not valid Unicode, then `tail`: a byte that is not UTF-8 on Unix, an unpaired surrogate
/// on Windows.
#[cfg(not(target_os = "macos"))]
fn not_unicode(stem: &str, tail: &str) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = stem.as_bytes().to_vec();
        bytes.push(0xff);
        bytes.extend(tail.as_bytes());
        std::ffi::OsString::from_vec(bytes)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = stem.encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(tail.encode_utf16());
        std::ffi::OsString::from_wide(&wide)
    }
}

/// Selects the row whose path starts with `prefix` on the sides it has.
#[cfg(not(target_os = "macos"))]
fn select_prefixed(app: &mut AppCore, view: ViewId, prefix: &str) -> hikasa_folder::NodeId {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let node = {
        let read = model.read();
        read.preorder()
            .map(|(id, _)| id)
            .find(|&id| {
                id != hikasa_folder::NodeId::ROOT && read.path(id).as_str().starts_with(prefix)
            })
            .unwrap_or_else(|| panic!("no row {prefix}"))
    };
    m.with_selection(|sel, _, present| {
        sel.items = Default::default();
        sel.items.insert(node, present(node));
        sel.current = Some(node);
    });
    node
}

/// The view a command opened last: the newest tab.
#[cfg(not(target_os = "macos"))]
fn newest(app: &mut TestApp) -> &mut crate::app::ViewModel {
    app.0
        .active_tab_mut(app.0.focused())
        .and_then(|t| t.model_mut())
        .expect("a view")
}

/// A9.30 (review F9): Quick Compare, Compare To and Open With ▸ Text Compare / Folder Compare take a name that is not
/// valid Unicode as it is on disk (OS strings, never `canonical()`'s replacement characters). Not on macOS: APFS keeps
/// names valid UTF-8 only, such a name cannot exist there.
#[cfg(not(target_os = "macos"))]
#[test]
fn a9_30_names_that_are_not_unicode_reach_the_views_and_quick_compare() {
    use crate::app::ViewModel;
    let world = World::new();
    let file = not_unicode("caf", ".txt");
    let folder = not_unicode("dir", "");
    for (side, text) in [("l", "left\n"), ("r", "right!\n")] {
        let root = world.path(&format!("work/{side}"));
        std::fs::create_dir_all(root.join(&folder)).expect("mkdir");
        std::fs::write(root.join(&file), text).expect("write");
        std::fs::write(root.join(&folder).join("in.txt"), text).expect("write");
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let (left, right) = (world.path("work/l"), world.path("work/r"));
    // Quick Compare: the pair is read, not "not found".
    select_prefixed(&mut app, view, "caf");
    app.execute(CommandId::QuickCompare);
    world.settle(&mut app);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let d = find::<QuickCompareDialog>(&mut app).expect("Quick Compare");
    assert_eq!(d.rows().len(), 1);
    assert!(d.rows()[0].result.is_ok(), "{:?}", d.rows()[0].result);
    press(&world, &mut app, ButtonRole::Cancel);
    // Compare To and Open With ▸ Text Compare: a Text Compare view of the exact paths, loaded.
    let w = app.win();
    for invocation in [
        Invocation::new(CommandId::CompareTo),
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::TextCompare),
        ),
    ] {
        app.activate(app.tabs().iter().position(|t| t.id == view).expect("view"));
        select_prefixed(&mut app, view, "caf");
        let tabs = app.tabs().len();
        app.0.execute(w, invocation.clone());
        world.settle(&mut app);
        assert_eq!(
            app.tabs().len(),
            tabs + 1,
            "{invocation:?}: {:?}",
            logged(&app, LogKind::Error)
        );
        let Some(ViewModel::Text(m)) = Some(newest(&mut app)) else {
            panic!("a Text Compare view");
        };
        assert_eq!(m.request().left, left.join(&file).into_os_string());
        assert_eq!(m.request().right, right.join(&file).into_os_string());
        assert!(m.response().is_some(), "{invocation:?}: loaded");
    }
    // Open With ▸ Folder Compare: the two folders as they are.
    app.activate(app.tabs().iter().position(|t| t.id == view).expect("view"));
    select_prefixed(&mut app, view, "dir");
    let tabs = app.tabs().len();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::FolderCompare),
        ),
    );
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 1);
    let Some(ViewModel::Folder(m)) = Some(newest(&mut app)) else {
        panic!("a Folder Compare view");
    };
    let prepared = m.prepared().expect("loaded");
    let specs = prepared.specs();
    assert_eq!(
        hikasa_session::api::spec_os_string(specs.left),
        left.join(&folder).into_os_string()
    );
    assert_eq!(prepared.model().read().len(), 2, "the base and in.txt");
}

// ---- A9.14: one operation per view; errors; beep ------------------------------------------------------------------

/// A second command is off while an operation runs; Stop cancels the execute job and its outcome reaches the log; a
/// refused plan is a warning notice and a log entry; an operation of 10 s or more beeps with the option, not before.
#[test]
fn a9_14_one_operation_errors_stop_and_the_beep() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[file_operations]\nconfirm_move = false\n[tweaks.file_operations]\nbeep_after_long_operations = true\n",
    )]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "only-left.txt");
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    app.execute(CommandId::MoveRight);
    poll_until(&world, &mut app, |a| folder_model(a, view).busy());
    for id in [
        CommandId::CopyRight,
        CommandId::MoveRight,
        CommandId::Touch,
        CommandId::CopyTo,
        CommandId::UpdateRight,
        CommandId::Compare,
        CommandId::NewFolder,
    ] {
        assert!(!app.enabled(id), "{id:?} during an operation");
    }
    // The plan's job passes the gate; the execute job is held there: Stop cancels it, and the outcome is logged.
    set_gate(&g, true);
    poll_until(&world, &mut app, |a| {
        folder_model(a, view).executed.is_some()
    });
    set_gate(&g, false);
    app.execute(CommandId::Stop);
    set_gate(&g, true);
    world.settle(&mut app);
    assert!(
        logged(&app, LogKind::Summary)
            .iter()
            .any(|l| l.contains("moved")),
        "{:?}",
        logged(&app, LogKind::Summary)
    );
    folder_model(&mut app, view).gate = None;
    // A refused plan (a name with a separator): a warning notice and its log entry.
    select(&mut app, view, "a.txt");
    app.execute(CommandId::Rename);
    folder_model(&mut app, view).set_inline_text("x/y.txt");
    let w = app.win();
    app.0.commit_inline_rename(w);
    world.settle(&mut app);
    assert!(
        notices(&app)
            .iter()
            .any(|(k, t, _)| *k == crate::notify::NoticeKind::Warning && t.starts_with("Rename")),
        "{:?}",
        notices(&app)
    );
    assert!(!logged(&app, LogKind::Warning).is_empty());
    // The beep (D-256): ≥ 10 s with the option, not below; an injected duration.
    assert!(app.0.beeps_after(Duration::from_secs(10)));
    assert!(!app.0.beeps_after(Duration::from_millis(9_999)));
    let plan = empty_plan(&world, &mut app, view);
    let outcome = Box::new(world.service.folder_op_dry_run(plan));
    for (secs, beeps) in [(11, true), (3, false)] {
        folder_model(&mut app, view).push_event(crate::folder_view::FolderEvent::Outcome {
            caption: "Move to Right".into(),
            outcome: outcome.clone(),
            elapsed: Duration::from_secs(secs),
        });
        let effects = app.0.handle(crate::app::UiMsg::Tick);
        assert_eq!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Beep(x) if *x == w)),
            beeps,
            "{secs} s: {effects:?}"
        );
    }
}

/// A plan with no item, for an injected outcome.
fn empty_plan(world: &World, app: &mut AppCore, view: ViewId) -> hikasa_folder::ops::OpPlan {
    let model = Arc::clone(folder_model(app, view).prepared().expect("p").model());
    let request = FolderOpRequest {
        op: OpRequest::Op(FolderOp::Copy {
            to: Side::Right,
            overwrite: OverwritePolicy::Skip,
        }),
        select: SelectRequest {
            paths: vec![("a.txt".into(), SideSet::LEFT)],
            ..SelectRequest::default()
        },
        read_only: Default::default(),
        overrides: OpOverrides::default(),
    };
    world
        .service
        .spawn(JobKind::FolderOp, "plan".into(), move |svc, job| {
            svc.folder_op_plan(&model, &request, job)
        })
        .wait()
        .expect("plan")
}
