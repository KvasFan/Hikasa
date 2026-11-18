//! M2 W13b acceptance A13.13 (D-364): the offset drag (whole pixels with the rest carried, Esc), nudges, Reset, the
//! ±2²⁰ stop, rotations and flips of the focused side and of the right picture from the difference pane, Auto Scale and
//! `aspect_ratio_differs`, Blend Toggle's view-only override and the slider, Swap Sides; P-PICJOBS — random
//! interleavings of drags, nudges, transforms, tolerance steps and mode changes with random job completion points end,
//! when idle, with the statistics `hikasa picture compare` gives for the final words, one option or geometry job at a
//! time.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use hikasa_picture::{DiffMode, MAX_OFFSET, Orientation, Pane};
use proptest::prelude::*;

use crate::commands::CommandId;
use crate::picture::viewport::{PaneRect, Zoom};
use crate::picture::{Gate, GateKind, PictureRequest, PictureViewModel};
use crate::tests::World;
use crate::tests_picture_open::{base, cli, held, open, pair, picture_model, png, wait};

const CONTENT: PaneRect = PaneRect::new(0.0, 0.0, 1200.0, 800.0);

fn diff_center(m: &PictureViewModel) -> (f64, f64) {
    m.slots()
        .iter()
        .find(|s| s.pane == Pane::Difference)
        .expect("difference pane")
        .picture
        .center()
}

/// A13.13: a drag of the difference pane moves the offset by `Δpoints × ppp / z` whole pixels, the rest carried to the
/// next move, at zooms 1/3, 1 and 2.5 and 1.5 pixels per point; Esc returns to the offset at its start. Mutation: a
/// drag at zoom 2 that moves twice as far.
#[test]
fn a13_13_offset_drag_with_carry_and_esc() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    for (zoom, ppp) in [(1.0 / 3.0, 1.0), (1.0, 1.0), (2.5, 1.0), (2.0, 1.5)] {
        m.set_ppp(ppp);
        m.arrange(CONTENT, 0.0);
        m.viewport.zoom = Zoom::Scale(zoom);
        m.reset_offset(&world.service);
        wait(&mut m, &world);
        let start = diff_center(&m);
        m.press(start, false);
        let mut moved = 0.0;
        for step in [1.0, 3.0, 0.5, 2.0, 7.0] {
            moved += step;
            m.drag_to(&world.service, (start.0 + moved, start.1 - moved / 2.0));
            let want_x = (moved * ppp / zoom).trunc() as i32;
            let want_y = (-moved / 2.0 * ppp / zoom).trunc() as i32;
            assert_eq!(
                m.options.geometry.offset,
                (want_x, want_y),
                "zoom {zoom} ppp {ppp} after {moved} points"
            );
        }
        m.release();
        wait(&mut m, &world);
        assert_eq!(
            m.comparison().expect("cmp").options().geometry.offset,
            m.options.geometry.offset,
            "the last offset landed"
        );
    }
    // Esc during a drag returns to the offset at its start.
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(1.0);
    m.set_ppp(1.0);
    let before = m.options.geometry.offset;
    let start = diff_center(&m);
    m.press(start, false);
    m.drag_to(&world.service, (start.0 + 9.0, start.1 + 4.0));
    assert_ne!(m.options.geometry.offset, before);
    assert!(m.offset_outline().is_some(), "the outline while dragging");
    assert!(m.escape(&world.service));
    wait(&mut m, &world);
    assert_eq!(m.options.geometry.offset, before);
    assert_eq!(
        m.comparison().expect("cmp").options().geometry.offset,
        before
    );
}

/// A13.13: nudges of 1 and 10 pixels; Reset; the ±2²⁰ stop with its message line.
#[test]
fn a13_13_nudges_reset_and_the_stop() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.nudge(&world.service, 1, 0, false);
    m.nudge(&world.service, 0, -1, true);
    assert_eq!(m.options.geometry.offset, (1, -10));
    assert_eq!(crate::picture::status::offset_text(&m), "Offset 1, -10");
    m.execute(&world.service, CommandId::OffsetReset);
    assert_eq!(m.options.geometry.offset, (0, 0));
    wait(&mut m, &world);
    // The stop.
    m.options.geometry.offset = (MAX_OFFSET - 4, 0);
    m.nudge(&world.service, 1, 0, true);
    assert_eq!(m.options.geometry.offset, (MAX_OFFSET, 0));
    assert!(
        m.take_requests()
            .iter()
            .any(|r| matches!(r, PictureRequest::Message(t) if t.contains("1,048,576")))
    );
    wait(&mut m, &world);
}

/// A13.13: the transforms act on the focused pane's picture, on the right one from the difference pane, composed as
/// `Orientation::then`; the statistics equal the CLI's with the same orientation words.
#[test]
fn a13_13_transforms_of_the_current_picture() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.focus = Pane::Left;
    m.execute(&world.service, CommandId::RotateRight);
    m.execute(&world.service, CommandId::RotateRight);
    m.focus = Pane::Difference;
    m.execute(&world.service, CommandId::FlipHorz);
    m.focus = Pane::Right;
    m.execute(&world.service, CommandId::RotateLeft);
    wait(&mut m, &world);
    let o = m.options.geometry.orientation;
    assert_eq!(o.left, Orientation::Cw90.then(Orientation::Cw90));
    assert_eq!(o.left, Orientation::Rot180);
    assert_eq!(o.right, Orientation::FlipH.then(Orientation::Ccw90));
    let direct = cli(&world, (&l, &r), |req| {
        req.geometry.left_orientation = Some(o.left.as_str().into());
        req.geometry.right_orientation = Some(o.right.as_str().into());
    });
    assert_eq!(
        m.comparison().expect("cmp").stats(),
        direct.comparison.stats()
    );
}

/// A13.13: Auto Scale of pictures of different shapes: the smaller side is scaled, `aspect_ratio_differs` is a
/// message line and an Info warning; the statistics equal the CLI's `--auto-scale`.
#[test]
fn a13_13_auto_scale_and_the_aspect_warning() {
    let world = World::new();
    let l = png(&world, "as/a.png", 40, 30, base);
    let r = png(&world, "as/b.png", 20, 20, base);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.execute(&world.service, CommandId::AutoScale);
    wait(&mut m, &world);
    assert!(m.comparison().expect("cmp").scaled().is_some());
    assert!(
        m.take_requests()
            .iter()
            .any(|r| matches!(r, PictureRequest::Message(_))),
        "the aspect warning is a message line"
    );
    let response = m.response(&world.service).expect("response");
    assert!(
        response
            .warnings
            .iter()
            .any(|w| w.code.as_str() == "aspect_ratio_differs")
    );
    let direct = cli(&world, (&l, &r), |req| req.rules.auto_scale = true);
    assert_eq!(response.comparison.stats(), direct.comparison.stats());
    assert_eq!(response.warnings, direct.warnings);
}

/// A13.13: Blend Toggle (F8) flips the shown percentage between 0 and 100 and never dirties the session; outside
/// Blend mode it switches to Blend at 0; the slider writes the percentage, clears the toggle and dirties the session.
#[test]
fn a13_13_blend_toggle_and_the_slider() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open(&world, &mut app, pair(&world));
    let dirty = |app: &crate::tests::TestApp| {
        app.tab(view)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.dirty)
    };
    app.execute(CommandId::BlendToggle);
    let m = picture_model(&mut app, view);
    assert_eq!(m.shown_mode(), DiffMode::Blend { percent: 0 });
    app.execute(CommandId::BlendToggle);
    assert_eq!(
        picture_model(&mut app, view).shown_mode(),
        DiffMode::Blend { percent: 100 }
    );
    assert!(!dirty(&app), "F8 is a quick look");
    let w = app.win();
    app.0.execute(
        w,
        crate::window::Invocation::with(
            CommandId::BlendPercent,
            crate::window::CommandArg::Number(30),
        ),
    );
    let m = picture_model(&mut app, view);
    assert_eq!(m.blend_toggle, None);
    assert_eq!(m.shown_mode(), DiffMode::Blend { percent: 30 });
    assert_eq!(m.request().rules.blend.as_deref(), Some("30"));
    assert!(dirty(&app), "the slider writes [rules.picture] blend");
}

/// A13.13 / A13.22: Swap Sides exchanges the pictures without decoding again and negates the offset; the statistics
/// equal the CLI's of the swapped words.
#[test]
fn a13_13_swap_sides_without_a_decode() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, calls) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.nudge(&world.service, 3, -2, false);
    m.focus = Pane::Left;
    m.execute(&world.service, CommandId::RotateRight);
    wait(&mut m, &world);
    let loads = calls.counters.load.load(Ordering::SeqCst);
    m.swap(&world.service);
    wait(&mut m, &world);
    assert_eq!(
        calls.counters.load.load(Ordering::SeqCst),
        loads,
        "no decode"
    );
    let g = m.options.geometry;
    assert_eq!(g.offset, (-3, 2));
    assert_eq!(g.orientation.right, Orientation::Cw90);
    let direct = cli(&world, (&r, &l), |req| {
        req.geometry.offset = Some("-3,2".into());
        req.geometry.right_orientation = Some("cw".into());
    });
    assert_eq!(
        m.comparison().expect("cmp").stats(),
        direct.comparison.stats()
    );
}

/// One change of a P-PICJOBS script.
#[derive(Debug, Clone)]
enum Change {
    Drag(f64, f64),
    Nudge(i64, i64, bool),
    Transform(u8, u8),
    Tolerance(u8),
    Mode(u8),
    Land(bool),
    Poll,
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        (-30.0f64..30.0, -30.0f64..30.0).prop_map(|(x, y)| Change::Drag(x, y)),
        (-1i64..=1, -1i64..=1, any::<bool>()).prop_map(|(x, y, c)| Change::Nudge(x, y, c)),
        (0u8..3, 0u8..4).prop_map(|(p, t)| Change::Transform(p, t)),
        (0u8..12).prop_map(Change::Tolerance),
        (0u8..3).prop_map(Change::Mode),
        any::<bool>().prop_map(Change::Land),
        Just(Change::Poll),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("HIKASA_HEAVY").map_or(24, |_| 10_000),
        ..ProptestConfig::default()
    })]

    /// P-PICJOBS. Mutation: the latest-wins rule drops the last change (the final statistics are a stale state's).
    #[test]
    fn p_picjobs(script in proptest::collection::vec(change(), 1..30)) {
        let world = World::new();
        let (l, r) = pair(&world);
        let gate = Arc::new(Gate::default());
        let (mut m, calls) = held(&world, (&l, &r), &gate);
        wait(&mut m, &world);
        m.arrange(CONTENT, 0.0);
        m.viewport.zoom = Zoom::Scale(4.0);
        let mut pressed = false;
        for c in script {
            match c {
                Change::Drag(dx, dy) => {
                    let s = diff_center(&m);
                    if !pressed {
                        m.press(s, false);
                        pressed = true;
                    }
                    m.drag_to(&world.service, (s.0 + dx, s.1 + dy));
                }
                Change::Nudge(x, y, ctrl) => m.nudge(&world.service, x, y, ctrl),
                Change::Transform(p, t) => {
                    m.focus = [Pane::Left, Pane::Right, Pane::Difference][usize::from(p)];
                    let id = [CommandId::RotateRight, CommandId::RotateLeft, CommandId::FlipHorz, CommandId::FlipVert][usize::from(t)];
                    m.execute(&world.service, id);
                }
                Change::Tolerance(t) => m.set_tolerance(&world.service, t),
                Change::Mode(k) => m.set_mode([DiffMode::Tolerance, DiffMode::Range, DiffMode::Blend { percent: 50 }][usize::from(k)]),
                Change::Land(hold) => {
                    if hold {
                        gate.close(GateKind::Geometry);
                        gate.close(GateKind::Options);
                    } else {
                        gate.open(GateKind::Geometry);
                        gate.open(GateKind::Options);
                    }
                }
                Change::Poll => {
                    m.poll(&world.service);
                }
            }
        }
        m.release();
        gate.open(GateKind::Geometry);
        gate.open(GateKind::Options);
        wait(&mut m, &world);
        prop_assert!(calls.counters.max_running.load(Ordering::SeqCst) <= 1);
        let o = m.options;
        let landed = *m.comparison().expect("cmp").options();
        prop_assert_eq!(landed, o, "the view ends on its last state");
        let direct = cli(&world, (&l, &r), |req| {
            req.geometry.offset = Some(format!("{},{}", o.geometry.offset.0, o.geometry.offset.1));
            req.geometry.left_orientation = Some(o.geometry.orientation.left.as_str().into());
            req.geometry.right_orientation = Some(o.geometry.orientation.right.as_str().into());
            req.rules.tolerance = Some(o.tolerance.to_string());
        });
        prop_assert_eq!(m.comparison().expect("cmp").stats(), direct.comparison.stats());
    }
}
