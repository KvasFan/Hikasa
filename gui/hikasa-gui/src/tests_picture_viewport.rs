//! M2 W13b acceptance A13.11 (D-362): the viewport of the Picture Compare view with fixed pane rectangles — Fit for
//! wide, tall, tiny and huge areas in each layout and with the difference pane hidden; Actual Size at 1, 1.5 and 2
//! pixels per point; the zoom steps and their ends; P-VIEWPORT (random wheel, pinch, key and pan scripts keep the area
//! point under the pointer under it, the panes' centres on one area point and 32 points of the area in every pane);
//! the pan rules per button and pane; full screen in and out.

use std::sync::Arc;

use hikasa_picture::Pane;
use proptest::prelude::*;

use crate::commands::CommandId;
use crate::picture::viewport::{
    FIT_MARGIN, KEEP, MAX_SCALE, MIN_SCALE, PaneRect, Viewport, ZOOM_STEPS, Zoom, fit_scale,
    step_from,
};
use crate::picture::{PictureLayout, PictureRequest};
use crate::tests::World;
use crate::tests_picture_open::{held, pair, wait};

/// The window's content rectangle of the bench (1,600 × 1,000 points less the chrome).
const CONTENT: PaneRect = PaneRect::new(0.0, 60.0, 1600.0, 900.0);

fn panes3() -> Vec<PaneRect> {
    vec![
        PaneRect::new(0.0, 0.0, 800.0, 450.0),
        PaneRect::new(800.0, 0.0, 800.0, 450.0),
        PaneRect::new(0.0, 450.0, 1600.0, 450.0),
    ]
}

/// A13.11: Fit — the largest scale at which the area fits every shown pane with the 8-point margin, small pictures
/// enlarged up to 64×, huge ones down to 1/256; in both layouts and with the difference pane hidden.
#[test]
fn a13_11_fit_in_every_layout() {
    let world = World::new();
    let gate = Arc::new(crate::picture::Gate::default());
    let (l, r) = pair(&world);
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    for layout in [PictureLayout::SideBySide, PictureLayout::Stacked] {
        for diff in [true, false] {
            m.layout = layout;
            m.shown[2] = diff;
            let slots = m.arrange(CONTENT, 0.0);
            assert_eq!(slots.len(), if diff { 3 } else { 2 });
            let rects: Vec<PaneRect> = slots.iter().map(|s| s.picture).collect();
            for area in [
                (4000.0, 3000.0),
                (300.0, 3000.0),
                (10.0, 8.0),
                (1048576.0, 10.0),
            ] {
                let s = fit_scale(&rects, area, 1.0);
                let want = rects
                    .iter()
                    .map(|p| {
                        ((p.w - 2.0 * FIT_MARGIN) / area.0).min((p.h - 2.0 * FIT_MARGIN) / area.1)
                    })
                    .fold(f64::INFINITY, f64::min)
                    .clamp(MIN_SCALE, MAX_SCALE);
                assert!((s - want).abs() < 1e-12, "{layout:?} {diff} {area:?}");
                let v = Viewport::default();
                for p in &rects {
                    let a = v.to_screen(*p, s, area, (0.0, 0.0));
                    let b = v.to_screen(*p, s, area, area);
                    if want > MIN_SCALE {
                        assert!(
                            a.0 >= p.x + FIT_MARGIN - 1e-6 && b.0 <= p.x + p.w - FIT_MARGIN + 1e-6
                        );
                        assert!(
                            a.1 >= p.y + FIT_MARGIN - 1e-6 && b.1 <= p.y + p.h - FIT_MARGIN + 1e-6
                        );
                    }
                    // Centred.
                    let c = v.to_area(*p, s, area, p.center());
                    assert!((c.0 - area.0 / 2.0).abs() < 1e-9 && (c.1 - area.1 / 2.0).abs() < 1e-9);
                }
            }
            assert_eq!(
                fit_scale(&rects, (4.0, 3.0), 1.0),
                MAX_SCALE,
                "tiny pictures fill the panes up to 64×"
            );
        }
    }
}

/// A13.11: Actual Size is one picture pixel per physical pixel at 1, 1.5 and 2 pixels per point; the steps of
/// Zoom In/Out and their ends.
#[test]
fn a13_11_actual_size_and_the_steps() {
    let world = World::new();
    let gate = Arc::new(crate::picture::Gate::default());
    let (l, r) = pair(&world);
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    for ppp in [1.0, 1.5, 2.0] {
        m.set_ppp(ppp);
        m.arrange(CONTENT, 0.0);
        m.execute(&world.service, CommandId::ZoomActual);
        assert_eq!(m.scale(), 1.0);
        let pane = m.slots()[0].picture;
        let a = m.viewport.to_screen(pane, 1.0, m.area(), (0.0, 0.0));
        let b = m.viewport.to_screen(pane, 1.0, m.area(), (1.0, 0.0));
        assert!(
            (b.0 - a.0 - 1.0 / ppp).abs() < 1e-12,
            "one pixel is 1/ppp points"
        );
    }
    m.set_ppp(1.0);
    // Every step up from the smallest to the largest, then the end.
    m.viewport.zoom = Zoom::Scale(MIN_SCALE);
    for want in &ZOOM_STEPS[1..] {
        m.execute(&world.service, CommandId::ZoomIn);
        assert_eq!(m.scale(), *want);
    }
    m.execute(&world.service, CommandId::ZoomIn);
    assert_eq!(m.scale(), MAX_SCALE, "the end");
    for want in ZOOM_STEPS[..24].iter().rev() {
        m.execute(&world.service, CommandId::ZoomOut);
        assert_eq!(m.scale(), *want);
    }
    assert_eq!(step_from(MIN_SCALE, false), None);
    assert_eq!(step_from(MAX_SCALE, true), None);
    assert_eq!(
        step_from(0.7, true),
        Some(1.0),
        "between steps: the next one"
    );
    assert_eq!(step_from(0.7, false), Some(2.0 / 3.0));
    // Fit again from the list.
    m.zoom_to(crate::picture::ZoomChoice::Fit);
    assert_eq!(m.viewport.zoom, Zoom::Fit);
    m.zoom_to(crate::picture::ZoomChoice::Percent(400));
    assert_eq!(m.scale(), 4.0);
}

/// One step of a P-VIEWPORT script.
#[derive(Debug, Clone)]
enum Op {
    Wheel {
        pane: usize,
        at: (f64, f64),
        up: bool,
    },
    Pinch {
        pane: usize,
        at: (f64, f64),
        factor: f64,
    },
    Key {
        up: bool,
    },
    Pan {
        dx: f64,
        dy: f64,
    },
    Fit,
}

fn op() -> impl Strategy<Value = Op> {
    let at = (0.0f64..1.0, 0.0f64..1.0);
    prop_oneof![
        (0usize..3, at.clone(), any::<bool>()).prop_map(|(pane, at, up)| Op::Wheel {
            pane,
            at,
            up
        }),
        (0usize..3, at, 0.5f64..2.0).prop_map(|(pane, at, factor)| Op::Pinch { pane, at, factor }),
        any::<bool>().prop_map(|up| Op::Key { up }),
        (-900.0f64..900.0, -600.0f64..600.0).prop_map(|(dx, dy)| Op::Pan { dx, dy }),
        Just(Op::Fit),
    ]
}

/// The invariants after every step: one area point at every pane's centre, 32 points of the area in every pane.
fn invariants(v: &Viewport, panes: &[PaneRect], area: (f64, f64)) {
    let s = v.scale(panes, area);
    let c0 = v.to_area(panes[0], s, area, panes[0].center());
    for p in panes {
        let c = v.to_area(*p, s, area, p.center());
        assert!(
            (c.0 - c0.0).abs() < 1e-9 && (c.1 - c0.1).abs() < 1e-9,
            "synchronized"
        );
        let a = v.to_screen(*p, s, area, (0.0, 0.0));
        let b = v.to_screen(*p, s, area, area);
        let shown_w = b.0.min(p.x + p.w) - a.0.max(p.x);
        let shown_h = b.1.min(p.y + p.h) - a.1.max(p.y);
        let k = s / v.ppp;
        let need_w = KEEP.min(area.0 * k).min(p.w);
        let need_h = KEEP.min(area.1 * k).min(p.h);
        assert!(shown_w >= need_w - 1e-6, "{shown_w} < {need_w}");
        assert!(shown_h >= need_h - 1e-6, "{shown_h} < {need_h}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("HIKASA_HEAVY").map_or(256, |_| 10_000),
        ..ProptestConfig::default()
    })]

    /// P-VIEWPORT: random zoom and pan scripts. After each wheel step about a pointer over the area the point under the
    /// pointer stays under it (within 1e-9) whenever the clamp has nothing to do; the invariants hold after every step.
    /// Mutation: `zoom_at` about the pane centre instead of the pointer.
    #[test]
    fn p_viewport(
        area in (1.0f64..20_000.0, 1.0f64..20_000.0),
        ppp in prop_oneof![Just(1.0), Just(1.5), Just(2.0)],
        ops in proptest::collection::vec(op(), 1..40),
    ) {
        let area = (area.0.round(), area.1.round());
        let panes = panes3();
        let mut v = Viewport { ppp, ..Viewport::default() };
        for op in ops {
            match op {
                Op::Wheel { pane, at, up } => {
                    let p = panes[pane];
                    let cursor = (p.x + at.0 * p.w, p.y + at.1 * p.h);
                    let s = v.scale(&panes, area);
                    let before = v.to_area(p, s, area, cursor);
                    if let Some(to) = step_from(s, up) {
                        // The unclamped centre the step would take.
                        let k = to / ppp;
                        let free = (before.0 - (cursor.0 - p.center().0) / k, before.1 - (cursor.1 - p.center().1) / k);
                        v.step_at(&panes, area, p, cursor, up);
                        let probe = Viewport { zoom: Zoom::Scale(to), center: free, ppp };
                        let ((x0, x1), (y0, y1)) = probe.center_range(&panes, area);
                        let inside = (x0..=x1).contains(&free.0) && (y0..=y1).contains(&free.1);
                        if inside {
                            let after = v.to_area(p, to, area, cursor);
                            prop_assert!((after.0 - before.0).abs() <= 1e-9 * before.0.abs().max(1.0));
                            prop_assert!((after.1 - before.1).abs() <= 1e-9 * before.1.abs().max(1.0));
                        }
                    }
                }
                Op::Pinch { pane, at, factor } => {
                    let p = panes[pane];
                    let cursor = (p.x + at.0 * p.w, p.y + at.1 * p.h);
                    let s = v.scale(&panes, area);
                    v.zoom_at(&panes, area, p, cursor, s * factor);
                    let now = v.scale(&panes, area);
                    prop_assert!((MIN_SCALE..=MAX_SCALE).contains(&now));
                }
                Op::Key { up } => {
                    let p = panes[2];
                    v.step_at(&panes, area, p, p.center(), up);
                }
                Op::Pan { dx, dy } => v.pan_by(&panes, area, (dx, dy)),
                Op::Fit => v.zoom = Zoom::Fit,
            }
            invariants(&v, &panes, area);
        }
    }
}

/// A13.11: the pan rules per button and pane — a primary drag pans in the left and right panes and moves the offset
/// in the difference pane; a middle drag pans in any pane; the wheel pans, Shift+wheel horizontally, Ctrl+wheel zooms
/// one step about the pointer.
#[test]
fn a13_11_pan_rules_per_button_and_pane() {
    let world = World::new();
    let gate = Arc::new(crate::picture::Gate::default());
    let (l, r) = pair(&world);
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.zoom_to(crate::picture::ZoomChoice::Percent(800));
    let center = |m: &crate::picture::PictureViewModel| m.viewport.center;
    let mid = |m: &crate::picture::PictureViewModel, pane: Pane| {
        m.slots()
            .iter()
            .find(|s| s.pane == pane)
            .expect("pane")
            .picture
            .center()
    };
    for pane in [Pane::Left, Pane::Right] {
        let start = mid(&m, pane);
        let c0 = center(&m);
        m.press(start, false);
        m.drag_to(&world.service, (start.0 + 16.0, start.1 + 8.0));
        m.release();
        let c1 = center(&m);
        assert!(
            (c0.0 - c1.0 - 2.0).abs() < 1e-9 && (c0.1 - c1.1 - 1.0).abs() < 1e-9,
            "{pane:?}: {c0:?} {c1:?}"
        );
        assert_eq!(m.options.geometry.offset, (0, 0));
    }
    // The difference pane: a primary drag moves the offset (16 points at 800 % = 2 pixels), a middle drag pans.
    let start = mid(&m, Pane::Difference);
    let c0 = center(&m);
    m.press(start, false);
    m.drag_to(&world.service, (start.0 + 16.0, start.1));
    m.release();
    assert_eq!(m.options.geometry.offset, (2, 0));
    assert_eq!(center(&m), c0);
    m.press(start, true);
    m.drag_to(&world.service, (start.0 - 8.0, start.1));
    m.release();
    assert!((center(&m).0 - c0.0 - 1.0).abs() < 1e-9);
    wait(&mut m, &world);
    // The wheel: vertical, Shift horizontal, Ctrl a zoom step about the pointer.
    let c0 = center(&m);
    let p = mid(&m, Pane::Left);
    m.wheel(p, (0.0, -24.0), false, false);
    assert!((center(&m).1 - c0.1 - 3.0).abs() < 1e-9);
    m.wheel(p, (0.0, -24.0), false, true);
    assert!((center(&m).0 - c0.0 - 3.0).abs() < 1e-9);
    m.wheel(p, (0.0, 1.0), true, false);
    assert_eq!(m.scale(), 12.0);
}

/// A13.11: full screen shows the focused pane alone without its header and asks the window for full screen; F11 or
/// Esc restores the layout.
#[test]
fn a13_11_full_screen_in_and_out() {
    let world = World::new();
    let gate = Arc::new(crate::picture::Gate::default());
    let (l, r) = pair(&world);
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.layout = PictureLayout::Stacked;
    m.focus = Pane::Difference;
    m.execute(&world.service, CommandId::FullScreen);
    assert_eq!(m.take_requests(), [PictureRequest::FullScreen(true)]);
    let slots = m.arrange(CONTENT, 30.0);
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].pane, Pane::Difference);
    assert_eq!(slots[0].picture, CONTENT, "no header, the whole view");
    assert!(m.escape(&world.service));
    assert_eq!(m.take_requests(), [PictureRequest::FullScreen(false)]);
    let slots = m.arrange(CONTENT, 30.0);
    assert_eq!(slots.len(), 3);
    assert_eq!(m.layout, PictureLayout::Stacked, "the layout is back");
    m.execute(&world.service, CommandId::FullScreen);
    m.execute(&world.service, CommandId::FullScreen);
    assert_eq!(
        m.take_requests(),
        [
            PictureRequest::FullScreen(true),
            PictureRequest::FullScreen(false)
        ]
    );
}
