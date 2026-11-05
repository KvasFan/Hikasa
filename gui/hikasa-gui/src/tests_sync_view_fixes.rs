//! M2 W9b after its review (§1.9.14, §1.9.15, §1.9.25 W9b notes 20 and after): a sync whose view ends, the overrides a
//! plan cannot use, the plan's state, the waited outcome's rows.

use std::sync::Arc;

use hikasa_core::ExitCode;
use hikasa_folder::sync::{SyncOptions, SyncPreset};
use hikasa_session::api::ViewOutcome;
use hikasa_session::ipc::ViewId;

use crate::commands::CommandId;
use crate::folder_view::Gate;
use crate::op_dialogs::{PlanConfirmDialog, YES_TO_ALL};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{gate, set_gate};
use crate::tests_sync_view::{confirm_all, find, fixture, open_sync, ticks};

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
