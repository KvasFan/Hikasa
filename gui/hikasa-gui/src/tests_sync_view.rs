//! M2 W9b (§1.9.14, §1.9.15; D-246…D-248): Synchronize from Folder Compare and the Folder Sync view through the models
//! (A9.15, A9.16, A9.17's GUI half). Each run is checked against the same sync made through the Service as `hikasa
//! folder sync --<preset> --yes` makes it on a twin tree; the view's plan against `folder_sync_plan` of the same
//! request (seeded property).

use std::collections::BTreeMap;
use std::sync::Arc;

use hikasa_core::{ExitCode, JobKind};
use hikasa_folder::NullObserver;
use hikasa_folder::ops::ConfirmMode;
use hikasa_folder::select::SideSet;
use hikasa_folder::sync::{SyncAction, SyncOptions, SyncPreset};
use hikasa_session::api::{
    FolderCompareRequest, FolderSyncRequest, SelectRequest, ViewOutcome, ViewRequest,
};
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};

use crate::app::{AppCore, ViewModel};
use crate::commands::{CommandId, Scope};
use crate::dialog::{ButtonRole, DialogModel};
use crate::folder_view::sync::SyncDialog;
use crate::op_dialogs::{PlanConfirmDialog, YES_TO_ALL};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{notices, open, put, select};
use crate::window::{CommandArg, Invocation};

// ---- helpers ------------------------------------------------------------------------------------------------------

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

fn trees(world: &World) -> [BTreeMap<String, String>; 2] {
    [tree(world, "l"), tree(world, "r")]
}

/// A newer file each way, orphans each side, a folder with a difference.
fn fixture(world: &World) {
    put(world, "l", "a.txt", "new left\n", 3_000);
    put(world, "r", "a.txt", "old\n", 1_000);
    put(world, "l", "b.txt", "old\n", 1_000);
    put(world, "r", "b.txt", "new right\n", 3_000);
    put(world, "l", "only-left.txt", "l\n", 1_000);
    put(world, "r", "only-right.txt", "r\n", 1_000);
    put(world, "l", "d/x.txt", "x\n", 2_000);
    put(world, "r", "d/x.txt", "x\n", 2_000);
    put(world, "l", "d/y.txt", "y left\n", 3_000);
}

fn find<T: DialogModel>(app: &mut TestApp) -> Option<&mut T> {
    let w = app.win();
    app.0.window_mut(w).and_then(|w| w.dialogs.find_mut::<T>())
}

fn ticks(world: &World, app: &mut TestApp) {
    for _ in 0..4 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    }
}

/// Opens a Folder Sync view of `work/l` and `work/r` with `options`.
fn open_sync(world: &World, app: &mut AppCore, options: SyncOptions) -> ViewId {
    for side in ["work/l", "work/r"] {
        std::fs::create_dir_all(world.path(side)).expect("mkdir");
    }
    let (left, right) = (world.path("work/l"), world.path("work/r"));
    let request = ViewRequest::Sync {
        request: FolderCompareRequest {
            left: left.clone().into_os_string(),
            right: right.clone().into_os_string(),
            ..FolderCompareRequest::default()
        },
        options,
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
    world.settle(app);
    for _ in 0..4 {
        app.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    }
    view
}

/// The same sync through the Service on a twin tree, as `hikasa folder sync --<preset> --yes` makes it
/// (`--just-selection` with `paths`).
fn cli_twin(world: &World, options: SyncOptions, paths: Option<Vec<(&str, SideSet)>>) -> ExitCode {
    let request = FolderCompareRequest {
        left: world.path("work/l").into_os_string(),
        right: world.path("work/r").into_os_string(),
        ..FolderCompareRequest::default()
    };
    let select = paths.map(|paths| SelectRequest {
        paths: paths.iter().map(|(p, s)| ((*p).to_owned(), *s)).collect(),
        ..SelectRequest::default()
    });
    world
        .service
        .spawn(JobKind::FolderSync, "twin".into(), move |svc, job| {
            let prepared = svc.folder_sync_prepare(&request, &options, job)?;
            svc.folder_run(&prepared, job, &NullObserver)?;
            let sync = FolderSyncRequest {
                options,
                select,
                ..FolderSyncRequest::default()
            };
            let mut plan = svc.folder_sync_plan(prepared.model(), &sync, job)?;
            let stand_in = plan.op.clone();
            let op = std::mem::replace(&mut plan.op, stand_in);
            plan.op = svc
                .folder_confirmation(&op, Some(ConfirmMode::YesToAll))
                .finish(op)
                .map_err(|_| hikasa_session::ApiError::Cancelled)?;
            Ok(svc.folder_sync_execute(plan, job)?.exit_code())
        })
        .wait()
        .expect("the twin ran")
}

/// Answers every question of a sync with Yes to All.
fn confirm_all(world: &World, app: &mut TestApp) {
    for _ in 0..4 {
        if find::<PlanConfirmDialog>(app).is_some() {
            crate::tests_folder_ops::press(world, app, YES_TO_ALL);
        }
        ticks(world, app);
    }
}

// ---- A9.15: Synchronize from Folder Compare ----------------------------------------------------------------------

/// Each of the five Synchronize commands, with and without a selection (*Just selection*), leaves the tree `folder sync
/// --<preset> --yes` leaves; a sync always asks its operation question.
#[test]
fn a9_15_each_synchronize_command_equals_the_cli() {
    let commands = [
        (CommandId::UpdateRight, SyncPreset::UpdateRight),
        (CommandId::UpdateLeft, SyncPreset::UpdateLeft),
        (CommandId::UpdateBoth, SyncPreset::UpdateBoth),
        (CommandId::MirrorRight, SyncPreset::MirrorRight),
        (CommandId::MirrorLeft, SyncPreset::MirrorLeft),
    ];
    for (id, preset) in commands {
        for just in [false, true] {
            let (gui, twin) = (World::new(), World::new());
            fixture(&gui);
            fixture(&twin);
            let mut app = gui.app(None);
            let view = open(&gui, &mut app);
            // The selected row is one the preset writes: newer on the left for the presets toward the right.
            let row = match preset {
                SyncPreset::UpdateLeft | SyncPreset::MirrorLeft => "b.txt",
                _ => "a.txt",
            };
            if just {
                select(&mut app, view, row);
            }
            app.execute(id);
            ticks(&gui, &mut app);
            let d = find::<SyncDialog>(&mut app).expect("the Synchronize preview");
            assert_eq!(d.just_selection, just, "{id:?}: preset with a selection");
            assert_eq!(d.options.preset, preset);
            assert!(d.list().is_some(), "{id:?} {just}: {:?}", d.message());
            assert!(
                d.buttons()
                    .iter()
                    .any(|b| b.role == ButtonRole::Ok && b.enabled),
                "{id:?} {just}: Run enabled: {:?}",
                d.list().map(|l| l.texts())
            );
            crate::tests_folder_ops::press(&gui, &mut app, ButtonRole::Ok);
            ticks(&gui, &mut app);
            assert!(
                find::<PlanConfirmDialog>(&mut app).is_some(),
                "{id:?}: the operation question; {:?} {:?}",
                notices(&app),
                crate::tests_folder_ops::top_title(&app)
            );
            confirm_all(&gui, &mut app);
            let options = SyncOptions {
                preset,
                ..SyncOptions::default()
            };
            let both = SideSet::LEFT | SideSet::RIGHT;
            cli_twin(&twin, options, just.then(|| vec![(row, both)]));
            assert_eq!(trees(&gui), trees(&twin), "{id:?} just {just}");
        }
    }
}

/// The preview's options: *Just selection* off without a selection, the recycle options and the confirmation mode
/// re-plan; a flat view has the submenu off; an empty source (`EmptySource`) shows core's message with Run off; an
/// unscanned folder stays unresolved.
#[test]
fn a9_15_options_flat_view_empty_source_and_unscanned_folders() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::MirrorRight);
    ticks(&world, &mut app);
    let d = find::<SyncDialog>(&mut app).expect("preview");
    assert!(!d.has_selection() && !d.just_selection);
    let before = d.plans;
    d.edit(|d| {
        d.just_selection = true;
        d.options.recycle = true;
        d.mode = ConfirmMode::NoToAll;
    });
    assert!(!d.just_selection, "nothing selected");
    ticks(&world, &mut app);
    let d = find::<SyncDialog>(&mut app).expect("preview");
    assert_eq!(d.plans, before + 1);
    assert!(
        d.list()
            .is_some_and(|l| l.texts().iter().any(|t| t.contains("trash"))),
        "the bin"
    );
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Cancel);
    // A flat view: no sync.
    app.execute(CommandId::DirFlatten);
    ticks(&world, &mut app);
    assert!(!app.enabled(CommandId::MirrorRight), "a flat view");
    app.execute(CommandId::DirFlatten);
    ticks(&world, &mut app);
    // An empty source: the message, Run off.
    for f in ["a.txt", "b.txt", "only-left.txt", "d/x.txt", "d/y.txt"] {
        std::fs::remove_file(world.path(&format!("work/l/{f}"))).expect("rm");
    }
    app.execute(CommandId::FullRefresh);
    ticks(&world, &mut app);
    app.execute(CommandId::MirrorRight);
    ticks(&world, &mut app);
    let d = find::<SyncDialog>(&mut app).expect("preview");
    assert!(d.message().is_some(), "EmptySource");
    assert!(
        !d.buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && b.enabled)
    );
    let _ = view;
}

// ---- A9.16: the Folder Sync view ---------------------------------------------------------------------------------

/// The view opens from the view request (IPC `OpenView { kind: folder_sync }`), from Home's *New* type, and plans once
/// the comparison ended; its scope is `Scope::Sync`; the actions per row equal `SyncPlan::rows`.
#[test]
fn a9_16_the_view_plans_and_shows_the_actions_of_the_plan() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let options = SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    };
    let view = open_sync(&world, &mut app, options);
    assert_eq!(app.scope(), Scope::Sync);
    app.execute(CommandId::ExpandAll);
    ticks(&world, &mut app);
    let m = folder_model(&mut app, view);
    let sync = m.sync().expect("a sync view");
    let plan = sync.plan().expect("planned");
    let window = m.window();
    for row in &window.rows {
        let planned = plan
            .rows
            .iter()
            .find(|r| r.node.0 == row.node)
            .map(|r| r.action);
        assert_eq!(row.sync.map(|a| a.action), planned, "row {}", row.node);
    }
    let counts = m.sync_counts().expect("counts");
    assert_eq!(
        counts.copy_right, 4,
        "a.txt, b.txt, only-left.txt, d/y.txt: {counts:?}"
    );
    assert!(app.enabled(CommandId::Run));
    // Info adds the plan's counts.
    app.execute(CommandId::Info);
    let info = find::<crate::info_dialog::InfoDialog>(&mut app).expect("Info");
    assert!(info.sections().iter().any(|s| s.title == "Sync plan"));
    assert!(info.title().starts_with("Folder Sync"));
    // From Home: the New type Folder Sync.
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::SessionNewList,
            CommandArg::SessionType(hikasa_session::sessions::SessionType::FolderSync),
        ),
    );
    world.settle(&mut app);
    if let Some(ViewModel::Home(home)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        home.panel.left = world.path("work/l").to_string_lossy().into_owned();
        home.panel.right = world.path("work/r").to_string_lossy().into_owned();
    }
    app.execute(CommandId::HomeCompare);
    ticks(&world, &mut app);
    assert_eq!(app.scope(), Scope::Sync, "Home's Folder Sync type");
}

/// The preset re-plans without a re-run; *Sync within archives* re-runs; an override of each selected row is written
/// by path and re-planned; an override the row cannot take comes back as a notice and is removed; conflicts are
/// walked.
#[test]
fn a9_16_presets_overrides_and_conflicts() {
    let world = World::new();
    fixture(&world);
    put(&world, "l", "c.txt", "conflict left\n", 2_000);
    put(&world, "r", "c.txt", "conflict right!\n", 2_000);
    let mut app = world.app(None);
    let view = open_sync(&world, &mut app, SyncOptions::default());
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    let plans = folder_model(&mut app, view).sync().expect("sync").plans;
    app.0.sync_preset(view, SyncPreset::MirrorLeft);
    ticks(&world, &mut app);
    let m = folder_model(&mut app, view);
    assert!(
        Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "no re-run"
    );
    assert_eq!(m.sync().expect("sync").plans, plans + 1);
    assert_eq!(
        m.sync().expect("sync").options.preset,
        SyncPreset::MirrorLeft
    );
    assert!(
        app.tabs()[app.active()]
            .session
            .as_ref()
            .is_some_and(|s| s.dirty)
    );
    // An override by path.
    select(&mut app, view, "a.txt");
    app.execute(CommandId::SyncLeave);
    ticks(&world, &mut app);
    let m = folder_model(&mut app, view);
    assert_eq!(
        m.sync().expect("sync").options.overrides.get("a.txt"),
        Some(&SyncAction::Leave)
    );
    let node = crate::tests_folder_ops::node_of(&mut app, view, "a.txt").expect("a.txt");
    let action = folder_model(&mut app, view)
        .sync_action(node)
        .expect("planned");
    assert!(action.overridden && action.action == SyncAction::Leave);
    // An override the row cannot take: a left orphan has nothing to copy left.
    select(&mut app, view, "only-left.txt");
    app.execute(CommandId::SyncCopyLeft);
    ticks(&world, &mut app);
    assert!(
        folder_model(&mut app, view)
            .sync()
            .expect("sync")
            .options
            .overrides
            .get("only-left.txt")
            .is_none(),
        "removed"
    );
    assert!(
        notices(&app)
            .iter()
            .any(|(_, t, _)| t.contains("only-left.txt")),
        "{:?}",
        notices(&app)
    );
    // A conflict: c.txt differs with the same time under Update Both.
    app.0.sync_preset(view, SyncPreset::UpdateBoth);
    ticks(&world, &mut app);
    assert!(app.enabled(CommandId::ConflictNext));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::ConflictNext);
    assert_eq!(
        crate::tests_folder_ops::selected_path(&mut app, view).as_deref(),
        Some("c.txt")
    );
    // Within archives: the comparison runs again.
    let model = Arc::clone(folder_model(&mut app, view).prepared().expect("p").model());
    let service = world.service.clone();
    folder_model(&mut app, view).edit_sync(&service, |o| o.within_archives = !o.within_archives);
    ticks(&world, &mut app);
    let m = folder_model(&mut app, view);
    assert!(
        !Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "re-run"
    );
}

/// Run Sync: the operation question with the counts, then the run; the tree equals `folder sync --session … --yes`;
/// the waited outcome is the last Run's (D-248); the session keeps `[rules.sync]` with the overrides.
#[test]
fn a9_16_run_sync_equals_the_cli_and_sets_the_outcome() {
    let (gui, twin) = (World::new(), World::new());
    fixture(&gui);
    fixture(&twin);
    let mut app = gui.app(None);
    let options = SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    };
    let view = open_sync(&gui, &mut app, options.clone());
    let service = gui.service.clone();
    // Before a Run: the comparison completed, exit 0.
    let outcome = folder_model(&mut app, view).outcome(&service);
    assert_eq!(
        outcome,
        ViewOutcome::Sync {
            last_run: None,
            complete: true
        }
    );
    select(&mut app, view, "b.txt");
    app.execute(CommandId::SyncLeave);
    ticks(&gui, &mut app);
    app.execute(CommandId::Run);
    ticks(&gui, &mut app);
    let question = find::<PlanConfirmDialog>(&mut app).expect("the operation question");
    assert!(
        question.subject().contains("copies right"),
        "{}",
        question.subject()
    );
    confirm_all(&gui, &mut app);
    let mut twin_options = options;
    twin_options
        .overrides
        .insert("b.txt".into(), SyncAction::Leave);
    assert_eq!(cli_twin(&twin, twin_options, None), ExitCode::Success);
    assert_eq!(trees(&gui), trees(&twin));
    let outcome = folder_model(&mut app, view).outcome(&service);
    assert_eq!(
        outcome,
        ViewOutcome::Sync {
            last_run: Some(ExitCode::Success),
            complete: true
        }
    );
    // The session the view saves keeps its options and override.
    let (request, _) = folder_model(&mut app, view).duplicate();
    let ViewRequest::Sync { options, .. } = request else {
        panic!("a sync request");
    };
    assert_eq!(options.overrides.get("b.txt"), Some(&SyncAction::Leave));
    assert_eq!(options.preset, SyncPreset::MirrorRight);
}

/// Swap Sides of a sync view mirrors its options (D-243): the same folders get the same actions.
#[test]
fn a9_16_swap_mirrors_the_options() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let mut options = SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    };
    options
        .overrides
        .insert("a.txt".into(), SyncAction::CopyRight);
    let _ = open_sync(&world, &mut app, options.clone());
    app.execute(CommandId::Swap);
    ticks(&world, &mut app);
    let view = app.tabs()[app.active()].id;
    let m = folder_model(&mut app, view);
    let swapped = &m.sync().expect("still a sync view").options;
    assert_eq!(*swapped, options.swapped());
    assert_eq!(swapped.preset, SyncPreset::MirrorLeft);
    assert_eq!(swapped.overrides.get("a.txt"), Some(&SyncAction::CopyLeft));
}

/// Property (A9.16): for seeded random trees, presets and overrides the view's plan equals `folder_sync_plan` of the
/// same request made directly (the dry runs' documents compared).
#[test]
fn p_the_view_plan_is_folder_sync_plan() {
    let cases: u64 = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(12, |n: u64| (n / 100).max(4));
    let names = ["a.txt", "b.txt", "d/c.txt", "d/e.txt", "f.txt"];
    for seed in 1..=cases {
        let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        let world = World::new();
        for name in names {
            match next(4) {
                0 => put(
                    &world,
                    "l",
                    name,
                    &format!("{name} l\n"),
                    1_000 + next(3) * 1_000,
                ),
                1 => put(
                    &world,
                    "r",
                    name,
                    &format!("{name} r\n"),
                    1_000 + next(3) * 1_000,
                ),
                2 => {
                    put(
                        &world,
                        "l",
                        name,
                        &format!("{name} l{}\n", next(2)),
                        1_000 + next(3) * 1_000,
                    );
                    put(
                        &world,
                        "r",
                        name,
                        &format!("{name} l{}\n", next(2)),
                        1_000 + next(3) * 1_000,
                    );
                }
                _ => {}
            }
        }
        let mut options = SyncOptions {
            preset: SyncPreset::ALL[next(5) as usize],
            ..SyncOptions::default()
        };
        if next(2) == 0 {
            let action = [
                SyncAction::Leave,
                SyncAction::CopyRight,
                SyncAction::DeleteLeft,
            ][next(3) as usize];
            options
                .overrides
                .insert(names[next(names.len() as u64) as usize].into(), action);
        }
        let mut app = world.app(None);
        let view = open_sync(&world, &mut app, options.clone());
        let m = folder_model(&mut app, view);
        let model = Arc::clone(m.prepared().expect("p").model());
        let read = model.read();
        let shown = m.sync().and_then(|s| s.plan()).map(|plan| {
            world
                .service
                .folder_sync_dry_run(plan.clone())
                .dto(&read, true)
        });
        drop(read);
        let effective = m.sync().expect("sync").options.clone();
        let request = FolderSyncRequest {
            options: effective,
            ..FolderSyncRequest::default()
        };
        let direct = world
            .service
            .spawn(JobKind::FolderSync, "direct".into(), {
                let model = Arc::clone(&model);
                move |svc, job| svc.folder_sync_plan(&model, &request, job)
            })
            .wait();
        let read = model.read();
        let direct = direct
            .ok()
            .map(|plan| world.service.folder_sync_dry_run(plan).dto(&read, true));
        assert_eq!(
            shown.map(|d| serde_json::to_value(d).expect("json")),
            direct.map(|d| serde_json::to_value(d).expect("json")),
            "seed {seed}: {options:?}"
        );
    }
}
