//! M2 W13b acceptance A13.10 (D-361) through the models: a Picture Compare view loads through `picture_load` and
//! `picture_diff` (held jobs), shows its headers and progress, then the panes; every row of §1.13.9's table with the
//! counters of the jobs and the pane generations (nothing else runs, nothing else changes); a failed side; Reload;
//! closing; the view kinds of A13.21 (IPC, Home, Open With, a folder row). Pictures are written by the test as PNG
//! through `hikasa_session::clipboard::encode_png_rgba` (the GUI never names `image`, D-373).

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{ContentKind, JobKind, Side};
use hikasa_picture::{DiffMode, Geometry, Pane, PictureColors, Rgba8};
use hikasa_session::api::{PictureCompareRequest, ViewOutcome};
use hikasa_session::ipc::{ViewId, ViewKind};

use crate::commands::{CommandId, Scope};
use crate::picture::{Calls, Gate, GateKind, PicturePhase, PictureViewModel};
use crate::tests::{TestApp, World};

/// A picture of `w` × `h` from `f` as RGBA bytes.
pub(crate) fn rgba(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&f(x, y));
        }
    }
    out
}

/// Writes a PNG of `w` × `h` from `f` under the world's work folder.
pub(crate) fn png(
    world: &World,
    rel: &str,
    w: u32,
    h: u32,
    f: impl Fn(u32, u32) -> [u8; 4],
) -> PathBuf {
    let bytes = hikasa_session::clipboard::encode_png_rgba(w, h, &rgba(w, h, f)).expect("png");
    let path = world.path(&format!("work/{rel}"));
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, bytes).expect("write");
    path
}

/// The base picture of the pairs: a gradient with a transparent corner.
pub(crate) fn base(x: u32, y: u32) -> [u8; 4] {
    let a = if x < 4 && y < 4 { 0 } else { 255 };
    [(x * 7) as u8, (y * 11) as u8, ((x + y) * 3) as u8, a]
}

/// A pair of 40 × 30 pictures: the right one differs in a blob (important) and in a few pixels by 3 (similar
/// under a tolerance of 5).
pub(crate) fn pair(world: &World) -> (PathBuf, PathBuf) {
    let l = png(world, "p/a.png", 40, 30, base);
    let r = png(world, "p/b.png", 40, 30, |x, y| {
        let mut p = base(x, y);
        if (10..16).contains(&x) && (8..12).contains(&y) {
            p = [255, 0, 0, 255];
        } else if x == 30 && y < 5 {
            p[0] = p[0].wrapping_add(3);
        }
        p
    });
    (l, r)
}

/// The model of a picture view.
pub(crate) fn picture_model(app: &mut TestApp, view: ViewId) -> &mut PictureViewModel {
    match app.tab_mut(view).expect("tab").model_mut().expect("live") {
        crate::app::ViewModel::Picture(m) => m,
        _ => panic!("not a picture view"),
    }
}

/// Opens two pictures as the command line does and waits for the comparison.
pub(crate) fn open(world: &World, app: &mut TestApp, (l, r): (PathBuf, PathBuf)) -> ViewId {
    let view = world.open(app, &l, &r);
    world.settle(app);
    assert_eq!(
        app.scope(),
        Scope::Picture,
        "two pictures open in Picture Compare"
    );
    view
}

/// A model built directly with a gate and counters (held jobs).
pub(crate) fn held(
    world: &World,
    (l, r): (&PathBuf, &PathBuf),
    gate: &Arc<Gate>,
) -> (PictureViewModel, Calls) {
    let calls = Calls {
        gate: Some(Arc::clone(gate)),
        ..Calls::default()
    };
    let request = PictureCompareRequest {
        left: l.as_os_str().to_owned(),
        right: r.as_os_str().to_owned(),
        ..PictureCompareRequest::default()
    };
    let m = PictureViewModel::with_calls(
        &world.service,
        request,
        Geometry::default(),
        DiffMode::Tolerance,
        [None, None],
        Arc::new(|| {}),
        Arc::new(hikasa_session::clipboard::OrderedClipboard::new(Arc::new(
            hikasa_session::clipboard::NoClipboard,
        ))),
        calls.clone(),
    );
    (m, calls)
}

/// Polls until no job of the model runs (bounded).
pub(crate) fn wait(m: &mut PictureViewModel, world: &World) {
    let began = std::time::Instant::now();
    loop {
        m.poll(&world.service);
        if !m.busy() {
            return;
        }
        assert!(
            began.elapsed() < std::time::Duration::from_secs(60),
            "jobs did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Polls until `done` holds (bounded).
pub(crate) fn wait_until(
    m: &mut PictureViewModel,
    world: &World,
    done: impl Fn(&PictureViewModel) -> bool,
) {
    let began = std::time::Instant::now();
    loop {
        m.poll(&world.service);
        if done(m) {
            return;
        }
        assert!(
            began.elapsed() < std::time::Duration::from_secs(60),
            "never reached"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The CLI's comparison of the same pair under `edit`'s words.
pub(crate) fn cli(
    world: &World,
    (l, r): (&PathBuf, &PathBuf),
    edit: impl FnOnce(&mut PictureCompareRequest),
) -> hikasa_session::api::PictureCompareResponse {
    let mut request = PictureCompareRequest {
        left: l.as_os_str().to_owned(),
        right: r.as_os_str().to_owned(),
        ..PictureCompareRequest::default()
    };
    edit(&mut request);
    let job = world
        .service
        .job(JobKind::PictureCompare, Arc::new(hikasa_core::NullProgress));
    world.service.picture_compare(&request, &job).expect("cli")
}

/// A13.10: the load in two held jobs — the headers and progress first, then the comparison; the verdict and the
/// statistics are `picture compare`'s.
#[test]
fn a13_10_load_then_compare_in_held_jobs() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    gate.close(GateKind::Load);
    gate.close(GateKind::Diff);
    let (mut m, calls) = held(&world, (&l, &r), &gate);
    m.poll(&world.service);
    assert_eq!(m.phase(), PicturePhase::Loading);
    assert!(m.header_lines(Pane::Left, &world.service)[1].contains("Decoding"));
    assert!(!m.jobs(ViewId(1)).is_empty(), "the progress shows");
    gate.open(GateKind::Load);
    wait_until(&mut m, &world, |m| m.phase() == PicturePhase::Comparing);
    // The File Info headers come from the decoded pictures before the comparison lands.
    let header = m.header_lines(Pane::Right, &world.service);
    assert!(header[1].starts_with("40\u{d7}30  png"), "{header:?}");
    assert!(m.comparison().is_none());
    gate.open(GateKind::Diff);
    wait(&mut m, &world);
    assert_eq!(m.phase(), PicturePhase::Ready);
    assert_eq!(calls.counters.snapshot(), [1, 1, 0, 0]);
    let direct = cli(&world, (&l, &r), |_| {});
    let view = m.response(&world.service).expect("response");
    assert_eq!(view.kind, ContentKind::ImportantDifferences);
    assert_eq!(view.kind, direct.kind);
    assert_eq!(view.comparison.stats(), direct.comparison.stats());
    assert_eq!(view.human_lines(), direct.human_lines());
}

/// One change of §1.13.9's table: what the model runs and which generations change.
struct Row {
    name: &'static str,
    /// Done before the counters and generations are read (a mode first).
    prepare: Option<fn(&World, &mut PictureViewModel)>,
    change: fn(&World, &mut PictureViewModel),
    /// Jobs started: `[load, diff, options, geometry]`.
    jobs: [usize; 4],
    /// Generations bumped: left, right, difference.
    bumped: [bool; 3],
}

/// A13.10: every row of §1.13.9's table on a fresh view: the jobs that run (counters) and the panes that change
/// (generations); nothing else runs, nothing else changes. Mutation: a mode change that bumps the left pane.
#[test]
fn a13_10_what_reruns_what() {
    let rows: &[Row] = &[
        Row {
            name: "tolerance (Tolerance mode)",
            prepare: None,
            change: |w, m| m.set_tolerance(&w.service, 5),
            jobs: [0, 0, 1, 0],
            bumped: [false, false, true],
        },
        Row {
            name: "tolerance (Range mode)",
            prepare: Some(|_, m| m.set_mode(DiffMode::Range)),
            change: |w, m| m.set_tolerance(&w.service, 5),
            jobs: [0, 0, 1, 0],
            bumped: [false, false, false],
        },
        Row {
            name: "mode",
            prepare: None,
            change: |_, m| m.set_mode(DiffMode::Range),
            jobs: [0, 0, 0, 0],
            bumped: [false, false, true],
        },
        Row {
            name: "blend percentage",
            prepare: None,
            change: |_, m| m.set_blend(30),
            jobs: [0, 0, 0, 0],
            bumped: [false, false, true],
        },
        Row {
            name: "F8",
            prepare: None,
            change: |_, m| m.blend_toggle(),
            jobs: [0, 0, 0, 0],
            bumped: [false, false, true],
        },
        Row {
            name: "Ignore Unimportant",
            prepare: None,
            change: |w, m| {
                m.execute(&w.service, CommandId::InsigIgnored);
            },
            jobs: [0, 0, 0, 0],
            bumped: [false, false, false],
        },
        // An offset that grows the area changes both sides' textures too (their edge texels and the overviews'
        // size: W13b review finding 8); `a13_10_an_offset_that_keeps_the_area_size` shows note 3's case.
        Row {
            name: "offset, the area's origin kept",
            prepare: None,
            change: |w, m| m.nudge(&w.service, 1, 1, false),
            jobs: [0, 0, 0, 1],
            bumped: [true, true, true],
        },
        Row {
            name: "offset, the area's origin moved",
            prepare: None,
            change: |w, m| m.nudge(&w.service, -1, 0, false),
            jobs: [0, 0, 0, 1],
            bumped: [true, true, true],
        },
        Row {
            name: "left orientation (a square keeps the origin)",
            prepare: None,
            change: |w, m| {
                m.focus = Pane::Left;
                m.execute(&w.service, CommandId::FlipHorz);
            },
            jobs: [0, 0, 0, 1],
            bumped: [true, false, true],
        },
        Row {
            name: "right orientation",
            prepare: None,
            change: |w, m| {
                m.focus = Pane::Right;
                m.execute(&w.service, CommandId::FlipVert);
            },
            jobs: [0, 0, 0, 1],
            bumped: [false, true, true],
        },
        Row {
            name: "the colours (Tolerance mode)",
            prepare: None,
            change: |_, m| {
                m.set_colors(PictureColors {
                    different: Rgba8::rgb(0x0000ff),
                    ..PictureColors::default()
                })
            },
            jobs: [0, 0, 0, 0],
            bumped: [false, false, true],
        },
        Row {
            name: "the colours (Range mode)",
            prepare: Some(|_, m| m.set_mode(DiffMode::Range)),
            change: |_, m| {
                m.set_colors(PictureColors {
                    different: Rgba8::rgb(0x0000ff),
                    ..PictureColors::default()
                });
            },
            jobs: [0, 0, 0, 0],
            bumped: [false, false, false],
        },
        Row {
            name: "checkerboard",
            prepare: None,
            change: |w, m| {
                m.execute(&w.service, CommandId::Checkerboard);
            },
            jobs: [0, 0, 0, 0],
            bumped: [false, false, false],
        },
        Row {
            name: "Reload",
            prepare: None,
            change: |w, m| {
                m.execute(&w.service, CommandId::Reload);
            },
            jobs: [1, 1, 0, 0],
            bumped: [true, true, true],
        },
        Row {
            name: "tone map",
            prepare: None,
            change: |w, m| {
                let mut request = m.request().clone();
                request.rules.tone_map = Some("reinhard".into());
                m.apply_rules(&w.service, request);
            },
            jobs: [1, 1, 0, 0],
            bumped: [true, true, true],
        },
    ];
    let world = World::new();
    // Square pictures: a flip keeps the area's origin.
    let l = png(&world, "sq/a.png", 24, 24, base);
    let r = png(&world, "sq/b.png", 24, 24, |x, y| {
        if x > 12 { [9, 9, 9, 255] } else { base(x, y) }
    });
    for row in rows {
        let gate = Arc::new(Gate::default());
        let (mut m, calls) = held(&world, (&l, &r), &gate);
        wait(&mut m, &world);
        if let Some(prepare) = row.prepare {
            prepare(&world, &mut m);
            wait(&mut m, &world);
        }
        let before_jobs = calls.counters.snapshot();
        let before = m.generations;
        (row.change)(&world, &mut m);
        wait(&mut m, &world);
        let after = calls.counters.snapshot();
        let jobs: Vec<usize> = (0..4).map(|k| after[k] - before_jobs[k]).collect();
        assert_eq!(jobs, row.jobs, "{}: jobs", row.name);
        let bumped: Vec<bool> = (0..3).map(|k| m.generations[k] != before[k]).collect();
        assert_eq!(bumped, row.bumped, "{}: generations", row.name);
        assert_eq!(m.phase(), PicturePhase::Ready, "{}", row.name);
    }
}

/// A13.10, §1.13.26 W13b note 3: an offset change that moves the area's origin with the right picture and keeps the
/// area's size (the right picture wider, defining both edges) leaves the right picture where it was in the area: its
/// pane keeps its generation; the left pane and the difference pane are new.
#[test]
fn a13_10_an_offset_that_keeps_the_area_size() {
    let world = World::new();
    let l = png(&world, "wide/a.png", 20, 12, base);
    let r = png(&world, "wide/b.png", 40, 12, base);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    // The right picture 10 pixels left of the left one: the area is the right picture, from x = −10.
    m.nudge(&world.service, -10, 0, false);
    wait(&mut m, &world);
    let before = (m.generations, m.comparison().expect("cmp").area());
    m.nudge(&world.service, -1, 0, false);
    wait(&mut m, &world);
    let c = m.comparison().expect("cmp");
    let a = c.area();
    assert_eq!(
        (a.width, a.height),
        (before.1.width, before.1.height),
        "the same size"
    );
    assert_eq!(
        a.x,
        before.1.x - 1,
        "the origin moved with the right picture"
    );
    let bumped: Vec<bool> = (0..3).map(|k| m.generations[k] != before.0[k]).collect();
    assert_eq!(bumped, [true, false, true]);
}

/// A13.10: a side that cannot be loaded shows its error in its header, the other side's facts stay, there is no
/// comparison and the outcome is `LoadFailed`; Reload loads again; closing cancels the jobs.
#[test]
fn a13_10_a_failed_side_and_reload() {
    let world = World::new();
    let (l, _) = pair(&world);
    let bad = world.write("work/p/broken.png", "not a picture");
    let mut app = world.app(None);
    let view = world.open(&mut app, &l, &bad);
    world.settle(&mut app);
    let m = picture_model(&mut app, view);
    assert_eq!(m.phase(), PicturePhase::Failed);
    assert!(m.side_error(Side::Right).is_some(), "{:?}", m.error());
    assert!(m.side_error(Side::Left).is_none());
    assert_eq!(m.outcome(&world.service), ViewOutcome::LoadFailed);
    // The file becomes a picture: Reload compares.
    std::fs::copy(&l, &bad).expect("copy");
    app.execute(CommandId::Reload);
    world.settle(&mut app);
    let m = picture_model(&mut app, view);
    assert_eq!(m.phase(), PicturePhase::Ready);
    let ViewOutcome::Text { result, .. } = m.outcome(&world.service) else {
        panic!("a verdict");
    };
    assert_eq!(result, hikasa_core::ContentResult::BinarySame);
    // Closing cancels a held job: the outcome is no verdict and nothing runs afterwards.
    let (l2, r2) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l2, &r2), &gate);
    wait(&mut m, &world);
    gate.close(GateKind::Options);
    m.set_tolerance(&world.service, 9);
    m.cancel();
    gate.open(GateKind::Options);
    wait(&mut m, &world);
    assert_eq!(m.outcome(&world.service), ViewOutcome::NoVerdict);
}

/// A13.21 (the Picture rows): two pictures open in Picture Compare from the command line and from a folder row;
/// Open With ▸ *Picture Compare*; Home and New Session offer the type; `TabDuplicate` keeps the view's state; the
/// outcome under the view's tolerance (`--wait`: 1, 2, 12, 13).
#[test]
fn a13_21_picture_views_open_from_everywhere() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l.clone(), r.clone()));
    // The verdict at close follows the view's tolerance and Ignore Unimportant.
    let m = picture_model(&mut app, view);
    let ViewOutcome::Text { result, .. } = m.outcome(&world.service) else {
        panic!()
    };
    assert_eq!(result, hikasa_core::ContentResult::ImportantDifferences);
    // TabDuplicate: the same comparison with the view's state.
    m.set_tolerance(&world.service, 4);
    world.settle(&mut app);
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), 2);
    let dup = app.tabs()[app.active()].id;
    assert_ne!(dup, view);
    assert_eq!(picture_model(&mut app, dup).options.tolerance, 4);
    // A folder view's row of two pictures opens in Picture Compare.
    use crate::tests_folder_ops::{open as open_folders, select};
    for side in ["l", "r"] {
        let to = world.path(&format!("work/{side}/a.png"));
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(if side == "l" { &l } else { &r }, &to).expect("copy");
    }
    let folder = open_folders(&world, &mut app);
    world.settle(&mut app);
    select(&mut app, folder, "a.png");
    let tabs = app.tabs().len();
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 1);
    assert_eq!(app.scope(), Scope::Picture, "a folder row of pictures");
    // Open With ▸ Picture Compare of the same row.
    let w = app.win();
    let index = app
        .0
        .window(w)
        .expect("window")
        .index_of(folder)
        .expect("folder tab");
    app.activate(index);
    select(&mut app, folder, "a.png");
    app.0.execute(
        w,
        crate::window::Invocation::with(
            CommandId::OpenWithList,
            crate::window::CommandArg::OpenWith(crate::window::OpenWithItem::PictureCompare),
        ),
    );
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 2);
    assert_eq!(app.scope(), Scope::Picture, "Open With ▸ Picture Compare");
    // Home and New Session offer the type.
    assert!(
        crate::menus::OPENABLE_TYPES
            .iter()
            .any(|(k, _)| *k == hikasa_session::sessions::SessionType::PictureCompare)
    );
    assert!(ViewKind::PictureCompare.is_served());
}
