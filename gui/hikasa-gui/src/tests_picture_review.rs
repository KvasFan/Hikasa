//! W13b review fixes (findings of the independent review): the picture view model's regression tests (the offset
//! outline, the bench's *undrawn*, stale texels after the area grows, the session centre, TabDuplicate's view state, a
//! failed reload's textures, and the later fixes).

use std::collections::HashMap;
use std::sync::Arc;

use hikasa_picture::{Pane, RgbaTile};

use crate::commands::CommandId;
use crate::picture::tiles::{TEXTURE_BUDGET, TextureSink, TileKey};
use crate::picture::viewport::{PaneRect, Zoom};
use crate::picture::{Gate, PicturePhase, PictureViewModel};
use crate::tests::World;
use crate::tests_picture_open::{base, held, open, pair, picture_model, png, wait};

const CONTENT: PaneRect = PaneRect::new(0.0, 0.0, 1200.0, 700.0);

#[derive(Default)]
struct Sink {
    textures: HashMap<TileKey, RgbaTile>,
}

impl TextureSink for Sink {
    fn upload(&mut self, key: TileKey, image: &RgbaTile) {
        self.textures.insert(key, image.clone());
    }
    fn evict(&mut self, key: TileKey) {
        self.textures.remove(&key);
    }
}

/// Frames with the renders done by hand until nothing is pending (bounded).
fn settle(m: &mut PictureViewModel, world: &World, sink: &mut Sink) {
    for _ in 0..64 {
        m.frame(&world.service, TEXTURE_BUDGET, sink);
        if let Some(source) = m.render_source() {
            m.tiles.render_now(&world.service, &source);
        }
        m.frame(&world.service, TEXTURE_BUDGET, sink);
        if !m.tiles.busy() {
            return;
        }
    }
    panic!("never settled");
}

fn diff_rect(m: &PictureViewModel) -> PaneRect {
    m.slots()
        .iter()
        .find(|s| s.pane == Pane::Difference)
        .expect("difference pane")
        .picture
}

/// F2: the right picture's outline while dragging sits where the right picture is drawn. `placement` is in area
/// coordinates already; `offset_outline` subtracts the area's origin once more, so at a negative offset the outline is
/// |dx| pixels off.
#[test]
fn offset_outline_at_a_negative_offset() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(4.0);
    m.viewport.center = (20.0, 15.0);
    m.focus = Pane::Difference;
    m.nudge(&world.service, -1, 0, true); // offset (-10, 0)
    wait(&mut m, &world);
    let c = Arc::clone(m.comparison().expect("cmp"));
    assert_eq!(c.options().geometry.offset, (-10, 0));
    let rect = diff_rect(&m);
    let start = rect.center();
    m.press(start, false);
    // 8 points at scale 4 = 2 pixels to the right: wanted (-8, 0), landed (-10, 0) while the job is held.
    gate.close(crate::picture::GateKind::Geometry);
    m.drag_to(&world.service, (start.0 + 8.0, start.1));
    assert_eq!(m.options.geometry.offset, (-8, 0));
    let placed = c.placement(hikasa_core::Side::Right).expect("placed");
    let want_area = (placed.x as f64 + 2.0, placed.y as f64);
    let want = m.viewport.to_screen(rect, m.scale(), m.area(), want_area);
    let got = m.offset_outline().expect("outline while dragging");
    gate.open(crate::picture::GateKind::Geometry);
    m.release();
    wait(&mut m, &world);
    assert!(
        (got.0 - want.0).abs() < 1e-6 && (got.1 - want.1).abs() < 1e-6,
        "outline at {:?}, the right picture is drawn at {want:?}",
        (got.0, got.1)
    );
}

/// F3: the bench's *undrawn* figure is 0 in a settled frame at a negative offset (it maps area points to left
/// coordinates and compares them with area-coordinate placements).
#[test]
fn undrawn_is_zero_when_settled_at_a_negative_offset() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m.focus = Pane::Difference;
    for _ in 0..3 {
        m.nudge(&world.service, -1, 0, true);
    }
    wait(&mut m, &world);
    assert_eq!(
        m.comparison().expect("cmp").options().geometry.offset,
        (-30, 0)
    );
    let mut sink = Sink::default();
    settle(&mut m, &world, &mut sink);
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        assert_eq!(m.undrawn(pane, 12), 0.0, "{pane:?} in a settled frame");
    }
}

/// F5: every resident texture of a pane's current generation equals core's render of its key on the current
/// comparison. The left pane keeps its generation when the right picture widens the area on the right (origin and
/// left placement unchanged), but its level-k texels at the area's old right edge were averaged over a block clipped to
/// the old area: the overview keeps stale pixels.
#[test]
fn resident_textures_equal_core_after_the_area_grows() {
    let world = World::new();
    let l = png(&world, "g/a.png", 1025, 40, base);
    let r = png(&world, "g/b.png", 1024, 40, base);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(0.5);
    m.viewport.center = (512.0, 20.0);
    let mut sink = Sink::default();
    settle(&mut m, &world, &mut sink);
    m.focus = Pane::Difference;
    m.nudge(&world.service, 1, 0, false);
    m.nudge(&world.service, 1, 0, false);
    wait(&mut m, &world);
    settle(&mut m, &world, &mut sink);
    let c = Arc::clone(m.comparison().expect("cmp"));
    assert_eq!(c.options().geometry.offset, (2, 0));
    let area = (c.area().width, c.area().height);
    let colors = m.colors();
    let mode = m.shown_mode();
    let mut checked = 0;
    for (key, image) in &sink.textures {
        if key.generation != m.generations[usize::from(key.pane)] {
            continue;
        }
        let pane = crate::picture::tiles::PANES[usize::from(key.pane)];
        let want = c.render_tile(pane, mode, &colors, key.rect(area), key.level);
        assert_eq!(
            (image.width, image.height),
            (want.width, want.height),
            "{key:?}"
        );
        assert!(image.rgba == want.rgba, "{key:?} holds stale pixels");
        checked += 1;
    }
    assert!(checked > 0);
}

/// F6: a session's centre applied by Session Settings' OK (`apply_session_view` on a landed view) is kept in
/// `session_center` and applied again by the next Reload (or Swap), throwing away where the user panned since.
#[test]
fn reload_keeps_the_panned_centre() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    let view = hikasa_session::api::picture_view_table(
        &m.options.geometry,
        &hikasa_session::api::PictureViewKeys {
            zoom: hikasa_picture::Zoom::Scale(8.0),
            center: Some((5.0, 5.0)),
            difference_pane: true,
        },
        true,
        false,
    );
    m.apply_session_view(&view);
    assert_eq!(m.viewport.center, (5.0, 5.0));
    let panes: Vec<PaneRect> = m.slots().iter().map(|s| s.picture).collect();
    let area = m.area();
    m.viewport.pan_by(&panes, area, (-40.0, -24.0));
    let panned = m.viewport.center;
    assert_ne!(panned, (5.0, 5.0));
    m.reload(&world.service);
    wait(&mut m, &world);
    assert_eq!(m.phase(), PicturePhase::Ready);
    assert_eq!(m.viewport.center, panned, "Reload moved the view back");
}

/// F7: `TabDuplicate` duplicates the view with its view state (§1.13.17): zoom, centre, difference pane, layout,
/// checkerboard. The duplicate is built without the source's `[view]` table, so it opens at the type's defaults.
#[test]
fn tab_duplicate_keeps_the_view_state() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l, r));
    {
        let m = picture_model(&mut app, view);
        m.arrange(PaneRect::new(0.0, 0.0, 1200.0, 800.0), 0.0);
        m.zoom_to(crate::picture::ZoomChoice::Percent(200));
        m.viewport.center = (12.0, 9.0);
        m.shown[2] = false;
        m.layout = crate::picture::PictureLayout::Stacked;
        m.checkerboard = false;
    }
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let dup = app.tabs()[app.active()].id;
    assert_ne!(dup, view);
    let m = picture_model(&mut app, dup);
    let got = (
        m.viewport.zoom,
        m.viewport.center,
        m.shown[2],
        m.layout,
        m.checkerboard,
    );
    assert_eq!(
        got,
        (
            Zoom::Scale(2.0),
            (12.0, 9.0),
            false,
            crate::picture::PictureLayout::Stacked,
            false
        ),
        "the duplicate's view state"
    );
}

/// F8: a reload that fails keeps the old comparison's textures resident (counted against the window's budget) with
/// nothing drawing them; they are never released while the view shows its error.
#[test]
fn a_failed_reload_releases_the_textures() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    let mut sink = Sink::default();
    settle(&mut m, &world, &mut sink);
    assert!(m.tiles.bytes() > 0);
    std::fs::write(&r, b"not a picture any more").expect("write");
    m.reload(&world.service);
    wait(&mut m, &world);
    assert_eq!(m.phase(), PicturePhase::Failed);
    for _ in 0..4 {
        m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    }
    assert!(m.draw_list(Pane::Left).is_empty());
    assert_eq!(
        m.tiles.bytes(),
        0,
        "{} bytes of textures held by a failed view",
        m.tiles.bytes()
    );
}

/// Finding 11: a tab no longer shown keeps only its overviews wanted: its pending tiles are no longer pending and a
/// batch that runs on renders nothing but overviews.
#[test]
fn a_hidden_tab_renders_only_its_overviews() {
    let world = World::new();
    let l = png(&world, "h/a.png", 1500, 900, base);
    let r = png(&world, "h/b.png", 1500, 900, base);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(1.0);
    m.viewport.center = (750.0, 450.0);
    let mut sink = Sink::default();
    // One frame: overviews and tiles wanted, nothing rendered yet.
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    assert!(m.tiles.wanted().iter().any(|(k, _)| !k.overview));
    m.hidden(&mut sink);
    assert!(m.tiles.wanted().iter().all(|(k, _)| k.overview));
    let before = m
        .tiles
        .counters
        .rendered
        .load(std::sync::atomic::Ordering::SeqCst);
    let source = m.render_source().expect("source");
    m.tiles.render_now(&world.service, &source);
    let rendered = m
        .tiles
        .counters
        .rendered
        .load(std::sync::atomic::Ordering::SeqCst)
        - before;
    assert_eq!(rendered, 3, "the three overviews alone");
}

/// Finding 12: closing a picture view nobody waits for never sleeps on the UI thread for a geometry job still
/// running (it waited up to a second for every close).
#[test]
fn closing_without_a_waiter_does_not_wait_for_a_job() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l.clone(), r.clone()));
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    gate.close(crate::picture::GateKind::Geometry);
    m.nudge(&world.service, 1, 0, false);
    assert!(m.busy());
    *picture_model(&mut app, view) = m;
    assert!(!world.shared.is_waited(view));
    let began = std::time::Instant::now();
    app.execute(CommandId::CloseTab);
    let took = began.elapsed();
    gate.open(crate::picture::GateKind::Geometry);
    assert!(app.tab_mut(view).is_none(), "the tab closed");
    // The old close slept until a whole second had passed while the job was held.
    assert!(
        took < std::time::Duration::from_secs(1),
        "the close took {took:?}"
    );
}

/// Finding 13: Ctrl+wheel zooms one step a notch (24 points, one line): small deltas of a touchpad or a
/// high-resolution wheel add up to a notch; a turn the other way starts again.
#[test]
fn ctrl_wheel_steps_once_a_notch() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(4.0);
    m.viewport.center = (20.0, 15.0);
    let p = diff_rect(&m).center();
    // Eleven deltas of 2 points: no notch yet.
    for _ in 0..11 {
        m.wheel(p, (0.0, 2.0), true, false);
    }
    assert_eq!(m.scale(), 4.0);
    // The twelfth makes 24 points: one step up.
    m.wheel(p, (0.0, 2.0), true, false);
    assert_eq!(m.scale(), 6.0);
    // 23 points down then 1 up: the turn starts again, nothing.
    m.wheel(p, (0.0, -23.0), true, false);
    m.wheel(p, (0.0, 1.0), true, false);
    assert_eq!(m.scale(), 6.0);
    // Two notches at once: two steps down.
    m.wheel(p, (0.0, -48.0), true, false);
    assert_eq!(m.scale(), 3.0);
}
