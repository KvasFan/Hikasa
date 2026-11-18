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

// ---- the review's surviving mutations (§1.13.26 W13b review fixes, R1…R21) ----------------------------------------

/// Frames at `budget` with the renders done by hand until nothing is pending (bounded).
fn settle_at(m: &mut PictureViewModel, world: &World, sink: &mut Sink, budget: u64) {
    for _ in 0..64 {
        m.frame(&world.service, budget, sink);
        if let Some(source) = m.render_source() {
            m.tiles.render_now(&world.service, &source);
        }
        m.frame(&world.service, budget, sink);
        if !m.tiles.busy() {
            return;
        }
    }
    panic!("never settled");
}

/// A model of a `w` × `h` pair (the right one with stripes of differences), arranged in `CONTENT`, the jobs off.
fn big(world: &World, w: u32, h: u32) -> PictureViewModel {
    let l = png(world, &format!("m{w}x{h}/a.png"), w, h, base);
    let r = png(world, &format!("m{w}x{h}/b.png"), w, h, |x, y| {
        let mut p = base(x, y);
        if x % 61 == 0 {
            p[1] ^= 0x55;
        }
        p
    });
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(world, (&l, &r), &gate);
    wait(&mut m, world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// R1, R2: a pane smaller than the 32 points kept keeps its own size of the area (the area's edge may reach its far
/// side); a scroll bar is the shown part's start and length.
#[test]
fn the_kept_part_of_a_small_pane_and_the_scroll_bars() {
    use crate::picture::viewport::Viewport;
    let v = Viewport {
        zoom: Zoom::Scale(1.0),
        center: (500.0, 500.0),
        ppp: 1.0,
    };
    let small = [PaneRect::new(0.0, 0.0, 20.0, 20.0)];
    let ((x0, x1), (y0, y1)) = v.center_range(&small, (1000.0, 1000.0));
    assert!(
        close(x0, 10.0) && close(x1, 990.0) && close(y0, 10.0) && close(y1, 990.0),
        "{:?}",
        ((x0, x1), (y0, y1))
    );
    let bars = v.scrollbars(PaneRect::new(0.0, 0.0, 100.0, 50.0), 1.0, (1000.0, 1000.0));
    let [Some((hs, hl)), Some((vs, vl))] = bars else {
        panic!("{bars:?}");
    };
    assert!(
        close(hs, 0.45) && close(hl, 0.1) && close(vs, 0.475) && close(vl, 0.05),
        "{bars:?}"
    );
}

/// R3, R4: a scroll bar dragged to a start shows the area from there (both axes); Home and End go to the top and the
/// bottom of the centre's range, PgDn and PgUp a pane height down and up.
#[test]
fn scroll_to_and_the_page_keys() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(48.0);
    m.viewport.center = (20.0, 15.0);
    m.focus = Pane::Left;
    m.scroll_to(Pane::Left, 0, 0.25);
    m.scroll_to(Pane::Left, 1, 0.5);
    let [Some((hs, _)), Some((vs, _))] = m.scrollbars(Pane::Left) else {
        panic!("bars at 4,800 %");
    };
    assert!(close(hs, 0.25) && close(vs, 0.5), "{hs} {vs}");
    let panes: Vec<PaneRect> = m.slots().iter().map(|s| s.picture).collect();
    let area = m.area();
    let ((_, _), (top, bottom)) = m.viewport.center_range(&panes, area);
    m.page(false, Some(true));
    assert!(close(m.viewport.center.1, bottom), "End");
    m.page(false, Some(false));
    assert!(close(m.viewport.center.1, top), "Home");
    let h = m.slots()[0].picture.h / 48.0;
    m.page(true, None);
    assert!(close(m.viewport.center.1, top + h), "PgDn");
    m.page(false, None);
    assert!(close(m.viewport.center.1, top), "PgUp");
}

/// R5: F6 and Shift+F6 cycle the shown panes forward and backward.
#[test]
fn f6_and_shift_f6_cycle_the_panes() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.focus = Pane::Left;
    let mut seen = Vec::new();
    for _ in 0..3 {
        m.execute(&world.service, CommandId::GoNext);
        seen.push(m.focus);
    }
    assert_eq!(seen, [Pane::Right, Pane::Difference, Pane::Left]);
    m.execute(&world.service, CommandId::GoPrev);
    assert_eq!(m.focus, Pane::Difference);
    m.shown[2] = false;
    m.focus = Pane::Left;
    m.execute(&world.service, CommandId::GoPrev);
    assert_eq!(m.focus, Pane::Right);
}

/// A sink that records each frame's uploads.
#[derive(Default)]
struct Recorder {
    textures: HashMap<TileKey, RgbaTile>,
    frame: Vec<u64>,
}

impl TextureSink for Recorder {
    fn upload(&mut self, key: TileKey, image: &RgbaTile) {
        self.frame
            .push(u64::from(image.width) * u64::from(image.height) * 4);
        self.textures.insert(key, image.clone());
    }
    fn evict(&mut self, key: TileKey) {
        self.textures.remove(&key);
    }
}

/// R6: a frame uploads at most 2 MiB, unless a single texture is larger (an overview alone).
#[test]
fn a_frame_uploads_at_most_two_mib() {
    let world = World::new();
    let mut m = big(&world, 600, 400);
    let mut sink = Recorder::default();
    let mut frames = Vec::new();
    for _ in 0..8 {
        m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
        frames.push(std::mem::take(&mut sink.frame));
        if let Some(source) = m.render_source() {
            m.tiles.render_now(&world.service, &source);
        }
    }
    let busiest = frames.iter().map(|f| f.len()).max().unwrap_or(0);
    assert!(busiest >= 2, "{frames:?}");
    for f in &frames {
        assert!(
            f.len() <= 1 || f.iter().sum::<u64>() <= 2 << 20,
            "a frame uploaded {f:?}"
        );
    }
}

/// R7: the rings are prefetch: when the visible set fits the budget but not with the rings, no ring is wanted.
#[test]
fn no_ring_when_it_does_not_fit() {
    let world = World::new();
    let mut m = big(&world, 1600, 1200);
    m.viewport.zoom = Zoom::Scale(1.0);
    m.viewport.center = (800.0, 600.0);
    let mut sink = Sink::default();
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    assert!(
        m.tiles.wanted().iter().any(|(_, visible)| !visible),
        "rings with the whole budget"
    );
    let visible = m.demand();
    m.frame(&world.service, visible, &mut sink);
    assert_eq!(m.tiles.level_of(m.scale()), 0, "no level shift");
    assert!(
        m.tiles.wanted().iter().all(|(_, visible)| *visible),
        "no ring within {visible} bytes"
    );
}

/// R8: a render a batch skipped (no longer wanted when its turn came) is no longer pending: wanted again, it is asked
/// again and drawn.
#[test]
fn a_skipped_render_is_asked_again() {
    let world = World::new();
    let mut m = big(&world, 1600, 1200);
    m.viewport.zoom = Zoom::Scale(1.0);
    m.viewport.center = (300.0, 600.0);
    let mut sink = Sink::default();
    // The batch of the first frame is held; the view moves away before it runs.
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    m.viewport.center = (1300.0, 600.0);
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    let source = m.render_source().expect("source");
    m.tiles.render_now(&world.service, &source);
    let skipped = m
        .tiles
        .counters
        .skipped
        .load(std::sync::atomic::Ordering::SeqCst);
    assert!(skipped > 0, "the first batch skipped the tiles left behind");
    // Back: the skipped tiles are wanted again and drawn.
    m.viewport.center = (300.0, 600.0);
    settle_at(&mut m, &world, &mut sink, TEXTURE_BUDGET);
    assert!(m.tiles.complete(), "every visible tile drawn");
}

/// R9: over the budget the panes draw one level coarser: the tiles of that level are drawn.
#[test]
fn the_shifted_level_is_drawn() {
    let world = World::new();
    let mut m = big(&world, 4000, 300);
    m.viewport.zoom = Zoom::Scale(1.0);
    m.viewport.center = (2000.0, 150.0);
    let budget = m.demand() - 1;
    let mut sink = Sink::default();
    settle_at(&mut m, &world, &mut sink, budget);
    assert_eq!(m.tiles.level_of(m.scale()), 1, "one level coarser");
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        let tiles: Vec<u8> = m
            .draw_list(pane)
            .iter()
            .filter(|i| !i.key.overview)
            .map(|i| i.key.level)
            .collect();
        assert!(
            !tiles.is_empty() && tiles.iter().all(|l| *l == 1),
            "{pane:?} draws {tiles:?}"
        );
    }
}

/// R10: while an offset drag waits for its job, the right pane draws its textures shifted by the part not landed
/// (the left pane's stay).
#[test]
fn the_right_textures_follow_the_drag() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(4.0);
    m.viewport.center = (20.0, 15.0);
    let mut sink = Sink::default();
    settle(&mut m, &world, &mut sink);
    let before = (m.draw_list(Pane::Left), m.draw_list(Pane::Right));
    gate.close(crate::picture::GateKind::Geometry);
    let start = diff_rect(&m).center();
    m.press(start, false);
    // 8 points at 400 % = 2 pixels.
    m.drag_to(&world.service, (start.0 + 8.0, start.1));
    let after = (m.draw_list(Pane::Left), m.draw_list(Pane::Right));
    gate.open(crate::picture::GateKind::Geometry);
    m.release();
    wait(&mut m, &world);
    assert_eq!(after.0, before.0, "the left pane stays");
    assert!(!before.1.is_empty() && before.1.len() == after.1.len());
    for (b, a) in before.1.iter().zip(&after.1) {
        assert!(
            close(a.rect.0 - b.rect.0, 8.0)
                && close(a.rect.2 - b.rect.2, 8.0)
                && close(a.rect.1, b.rect.1),
            "{b:?} → {a:?}"
        );
    }
}

/// R12: when the offset moves the area's origin the centre moves with it: the left picture stays still on screen.
#[test]
fn the_left_picture_stays_when_the_origin_moves() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(4.0);
    m.viewport.center = (20.0, 15.0);
    let left_at_centre = |m: &PictureViewModel| {
        let a = m.comparison().expect("cmp").area();
        (
            m.viewport.center.0 + a.x as f64,
            m.viewport.center.1 + a.y as f64,
        )
    };
    let before = left_at_centre(&m);
    m.nudge(&world.service, -3, -2, false);
    wait(&mut m, &world);
    let a = m.comparison().expect("cmp").area();
    assert_eq!((a.x, a.y), (-3, -2), "the right picture defines the origin");
    assert_eq!(left_at_centre(&m), before);
}

/// R13: the duplicate's request is complete: its words alone give the view's geometry (what core reads when the
/// request is checked, sent over IPC or written to a session).
#[test]
fn the_duplicate_words_carry_the_geometry() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.nudge(&world.service, 3, -2, false);
    m.focus = Pane::Right;
    m.execute(&world.service, CommandId::FlipHorz);
    wait(&mut m, &world);
    let (
        hikasa_session::api::ViewRequest::Picture {
            request, geometry, ..
        },
        _,
    ) = m.duplicate()
    else {
        panic!("a picture request");
    };
    let settings = world.service.check_picture_words(&request).expect("words");
    assert_eq!(geometry, m.options.geometry);
    assert_eq!(settings.options.geometry, m.options.geometry);
}

/// R15: every figure row of Info is its statistic (a tolerance that makes the pair's differences similar).
#[test]
fn info_rows_are_the_statistics() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.set_tolerance(&world.service, 5);
    wait(&mut m, &world);
    let response = m.response(&world.service).expect("response");
    let s = *response.comparison.stats();
    assert!(s.similar > 0 && s.similar != s.different, "{s:?}");
    let dialog = crate::picture::info::PictureInfoDialog::new(
        hikasa_session::ipc::ViewId(1),
        &response,
        Arc::new(hikasa_session::clipboard::NoClipboard),
    );
    let counted = |n: u64| format!("{n} ({:.4} %)", s.percent(n));
    let rows: Vec<(String, String)> = dialog.sections()[0]
        .rows
        .iter()
        .map(|r| (r.label.clone(), r.values.join(" ")))
        .collect();
    let want = [
        ("Covered pixels", s.pixels.to_string()),
        ("Same pixels", counted(s.same)),
        ("Similar pixels", counted(s.similar)),
        ("Different pixels", counted(s.different)),
        ("Left only", counted(s.left_only)),
        ("Right only", counted(s.right_only)),
        ("Maximum distance", s.max_distance.to_string()),
    ];
    for (k, (label, value)) in want.iter().enumerate() {
        assert_eq!(rows[k], ((*label).to_owned(), value.clone()));
    }
}

/// R21: the probe is the area pixel under the pointer at a scale where a point is a quarter pixel.
#[test]
fn the_probe_at_four_hundred_percent() {
    let world = World::new();
    let (l, r) = pair(&world);
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(&world, (&l, &r), &gate);
    wait(&mut m, &world);
    m.arrange(CONTENT, 0.0);
    m.viewport.zoom = Zoom::Scale(4.0);
    m.viewport.center = (20.0, 15.0);
    let rect = diff_rect(&m);
    // Just left of the boundary between pixels 9 and 10 (a tenth of a pixel, less than a point).
    let p = m.viewport.to_screen(rect, 4.0, m.area(), (9.9, 5.5));
    m.pointer_moved(p);
    let probe = m.probe().copied().expect("a probe");
    assert_eq!((probe.area_x, probe.area_y), (9, 5));
}
