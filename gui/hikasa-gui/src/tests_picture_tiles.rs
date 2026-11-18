//! M2 W13b acceptance A13.12 (D-363): tiles and textures through the model and a fake texture sink — the level of a
//! scale; tile keys and generations; overviews first, never evicted while shown; at most 8 uploads a frame, visible
//! first; the 64 MiB budget (here a small one) never exceeded and never paid with a visible tile; a hidden tab drops
//! its tiles; the old generation drawn only where no new tile covers it; P-TILESCREEN — every sampled pixel of every
//! pane drawn by a fake painter equals `render_tile`'s pixel of its level, with the checkerboard composited where the
//! alpha is below 255 (at 100 % equal to `composite_checkerboard`).

use std::collections::HashMap;
use std::sync::Arc;

use hikasa_picture::{DiffMode, Pane, PictureColors, RgbaTile, TileRect, composite_checkerboard};
use proptest::prelude::*;

use crate::picture::tiles::{
    DrawItem, MAX_LEVEL, TEXTURE_BUDGET, TILE, TextureSink, TileKey, UPLOADS_PER_FRAME, level_for,
    overview_level,
};
use crate::picture::viewport::{PaneRect, Zoom};
use crate::picture::{Gate, PictureViewModel};
use crate::tests::World;
use crate::tests_picture_open::{base, held, png, wait};

/// The content rectangle of the tests (three panes).
const CONTENT: PaneRect = PaneRect::new(0.0, 0.0, 1200.0, 700.0);

/// A recording texture sink.
#[derive(Default)]
pub(crate) struct FakeSink {
    pub(crate) textures: HashMap<TileKey, RgbaTile>,
    pub(crate) uploads: Vec<Vec<TileKey>>,
    pub(crate) evicted: Vec<TileKey>,
    frame: Vec<TileKey>,
}

impl FakeSink {
    fn end_frame(&mut self) {
        self.uploads.push(std::mem::take(&mut self.frame));
    }

    fn bytes(&self) -> u64 {
        self.textures
            .values()
            .map(|t| u64::from(t.width) * u64::from(t.height) * 4)
            .sum()
    }
}

impl TextureSink for FakeSink {
    fn upload(&mut self, key: TileKey, image: &RgbaTile) {
        self.textures.insert(key, image.clone());
        self.frame.push(key);
    }

    fn evict(&mut self, key: TileKey) {
        self.textures.remove(&key);
        self.evicted.push(key);
    }
}

/// A view of a `w` × `h` pair with the tile jobs run by hand.
fn view(world: &World, w: u32, h: u32) -> PictureViewModel {
    let l = png(world, &format!("t{w}/a.png"), w, h, base);
    let r = png(world, &format!("t{w}/b.png"), w, h, |x, y| {
        if (x / 37 + y / 23) % 5 == 0 {
            [200, 10, 10, 128]
        } else {
            base(x, y)
        }
    });
    let gate = Arc::new(Gate::default());
    let (mut m, _) = held(world, (&l, &r), &gate);
    wait(&mut m, world);
    m.tiles.run_jobs = false;
    m.arrange(CONTENT, 0.0);
    m
}

/// One frame: the model's frame, the renders done by hand, a second frame that uploads (as the window's next frames).
fn frames(m: &mut PictureViewModel, world: &World, sink: &mut FakeSink, budget: u64) {
    m.frame(&world.service, budget, sink);
    sink.end_frame();
    if let Some(source) = m.render_source() {
        m.tiles.render_now(&world.service, &source);
    }
    m.frame(&world.service, budget, sink);
    sink.end_frame();
}

/// Frames until nothing is left to upload (bounded).
fn settle(m: &mut PictureViewModel, world: &World, sink: &mut FakeSink, budget: u64) {
    for _ in 0..64 {
        frames(m, world, sink, budget);
        if sink.uploads.last().is_some_and(Vec::is_empty) && !m.tiles.busy() {
            return;
        }
    }
    panic!("the tiles never settled");
}

/// A13.12: the level of a scale (normative table).
#[test]
fn a13_12_level_of_a_scale() {
    for (scale, level) in [
        (64.0, 0),
        (1.0, 0),
        (0.75, 0),
        (0.5, 1),
        (0.3, 1),
        (0.25, 2),
        (1.0 / 3.0, 1),
        (0.125, 3),
        (1.0 / 256.0, 8),
        (1.0 / 300.0, MAX_LEVEL),
    ] {
        assert_eq!(level_for(scale), level, "{scale}");
    }
    assert_eq!(overview_level((4000, 3000)), Some(2));
    assert_eq!(overview_level((1024, 1024)), Some(0));
    assert_eq!(overview_level((1025, 10)), Some(1));
    assert_eq!(overview_level((300_000, 10)), None);
}

/// A13.12: the overviews come first; then at most 8 uploads a frame, the visible tiles before the ring; nothing is
/// uploaded twice; the model's count equals the sink's.
#[test]
fn a13_12_overviews_first_then_eight_a_frame() {
    let world = World::new();
    let mut m = view(&world, 1500, 1100);
    m.zoom_to(crate::picture::ZoomChoice::Actual);
    let mut sink = FakeSink::default();
    settle(&mut m, &world, &mut sink, TEXTURE_BUDGET);
    let uploads: Vec<&Vec<TileKey>> = sink.uploads.iter().filter(|f| !f.is_empty()).collect();
    assert!(uploads.len() >= 3, "many frames: {}", uploads.len());
    // The three overviews come first (an overview may take a frame of its own: 2 MiB a frame).
    let order: Vec<TileKey> = uploads.iter().flat_map(|f| f.iter().copied()).collect();
    assert!(order.iter().take(3).all(|k| k.overview), "{order:?}");
    for frame in &uploads {
        assert!(frame.len() <= 8, "at most 8 uploads a frame");
    }
    assert_eq!(UPLOADS_PER_FRAME, 8);
    // Visible before the ring, each pane's order by distance to its centre (the order the model wanted).
    let wanted = m.tiles.wanted().to_vec();
    let pos = |k: &TileKey| order.iter().position(|o| o == k);
    let last_visible = wanted
        .iter()
        .filter(|(_, v)| *v)
        .filter_map(|(k, _)| pos(k))
        .max()
        .expect("visible");
    let first_ring = wanted
        .iter()
        .filter(|(_, v)| !*v)
        .filter_map(|(k, _)| pos(k))
        .min();
    if let Some(first_ring) = first_ring {
        assert!(last_visible < first_ring, "visible first");
    }
    let unique: std::collections::HashSet<_> = order.iter().collect();
    assert_eq!(unique.len(), order.len(), "nothing uploaded twice");
    assert_eq!(
        m.tiles.bytes(),
        sink.bytes(),
        "the model counts what the sink holds"
    );
    // A hidden tab drops its tiles and keeps its overviews.
    m.hidden(&mut sink);
    assert!(sink.textures.keys().all(|k| k.overview));
    assert_eq!(sink.textures.len(), 3);
    assert_eq!(m.tiles.bytes(), sink.bytes());
}

/// A13.12: a new generation of the difference pane (a tolerance change): until its overview is uploaded the old
/// textures are drawn, but never where a resident tile of the new generation is; once it is, the old ones are gone.
/// Mutation: a tile key without the colours' stamp (a colour change would draw the old tiles forever).
#[test]
fn a13_12_old_generation_until_covered() {
    let world = World::new();
    let mut m = view(&world, 1500, 1100);
    m.zoom_to(crate::picture::ZoomChoice::Actual);
    let mut sink = FakeSink::default();
    settle(&mut m, &world, &mut sink, TEXTURE_BUDGET);
    let old = m.generations[2];
    m.set_colors(PictureColors {
        different: hikasa_picture::Rgba8::rgb(0x00ff00),
        ..PictureColors::default()
    });
    assert_ne!(
        m.generations[2], old,
        "the colours' stamp is in the difference pane's keys"
    );
    // One frame: the new keys are wanted, nothing new is resident yet: the old textures are all drawn.
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    let list = m.draw_list(Pane::Difference);
    assert!(!list.is_empty() && list.iter().all(|i| i.key.generation == old));
    // Render; upload one frame (8): the overview first, so the old generation goes at once.
    if let Some(source) = m.render_source() {
        m.tiles.render_now(&world.service, &source);
    }
    m.frame(&world.service, TEXTURE_BUDGET, &mut sink);
    let list = m.draw_list(Pane::Difference);
    assert!(
        list.iter().all(|i| i.key.generation == m.generations[2]),
        "{list:?}"
    );
    assert!(
        !sink
            .textures
            .keys()
            .any(|k| k.pane == 2 && k.generation == old)
    );
}

/// The topmost texture under a screen point and the texel there: `(key, x, y)`.
fn texel_at(list: &[DrawItem], p: (f64, f64), sink: &FakeSink) -> Option<(TileKey, u32, u32)> {
    list.iter().rev().find_map(|item| {
        let (x0, y0, x1, y1) = item.rect;
        if p.0 < x0 || p.0 >= x1 || p.1 < y0 || p.1 >= y1 {
            return None;
        }
        let image = sink.textures.get(&item.key)?;
        let (u0, v0, u1, v1) = item.uv;
        let u = u0 + (p.0 - x0) / (x1 - x0) * (u1 - u0);
        let v = v0 + (p.1 - y0) / (y1 - y0) * (v1 - v0);
        let tx = (u * f64::from(image.width)).floor();
        let ty = (v * f64::from(image.height)).floor();
        // Points on a texel's edge are not sampled (nearest sampling is undefined there).
        let fx = u * f64::from(image.width) - tx;
        let fy = v * f64::from(image.height) - ty;
        if !(1e-6..1.0 - 1e-6).contains(&fx) || !(1e-6..1.0 - 1e-6).contains(&fy) {
            return Some((item.key, u32::MAX, u32::MAX));
        }
        Some((item.key, tx as u32, ty as u32))
    })
}

/// The fake painter's pixel: the texel over the checkerboard cell of the point (8 physical pixels anchored at the
/// area's top-left corner on screen), composited as core composites.
fn painted(
    texel: [u8; 4],
    p: (f64, f64),
    anchor: (f64, f64),
    cell: f64,
    colors: &PictureColors,
) -> [u8; 4] {
    let a = u32::from(texel[3]);
    if a == 255 {
        return texel;
    }
    let cx = ((p.0 - anchor.0) / cell).floor() as i64;
    let cy = ((p.1 - anchor.1) / cell).floor() as i64;
    let bg = if (cx + cy).rem_euclid(2) == 0 {
        colors.checker_light
    } else {
        colors.checker_dark
    }
    .to_array();
    let r = |n: u32, d: u32| (2 * n + d) / (2 * d);
    let mut out = [0u8; 4];
    for c in 0..3 {
        out[c] = r(u32::from(texel[c]) * a + u32::from(bg[c]) * (255 - a), 255) as u8;
    }
    out[3] = (a + r(u32::from(bg[3]) * (255 - a), 255)) as u8;
    out
}

/// One P-TILESCREEN case: every sampled pixel of every pane equals `render_tile`'s pixel of the texture's level at the
/// area point under it (checkerboard composited where the alpha is below 255).
fn tile_screen(world: &World, m: &mut PictureViewModel, samples: u32) {
    let mut sink = FakeSink::default();
    settle(m, world, &mut sink, TEXTURE_BUDGET);
    let c = Arc::clone(m.comparison().expect("comparison"));
    let colors = m.colors();
    let mode = m.shown_mode();
    let scale = m.scale();
    let area = m.area();
    let mut checked = 0;
    for slot in m.slots().to_vec() {
        let pane = slot.pane;
        let list = m.draw_list(pane);
        let (anchor, cell) = m.checker_anchor(pane).expect("anchor");
        let r = slot.picture;
        for j in 0..samples {
            for i in 0..samples {
                let p = (
                    r.x + (f64::from(i) + 0.5) * r.w / f64::from(samples),
                    r.y + (f64::from(j) + 0.5) * r.h / f64::from(samples),
                );
                let a = m.viewport.to_area(r, scale, area, p);
                let inside = a.0 >= 0.0 && a.1 >= 0.0 && a.0 < area.0 && a.1 < area.1;
                let Some((key, tx, ty)) = texel_at(&list, p, &sink) else {
                    assert!(!inside, "{pane:?}: an undrawn point of the area at {p:?}");
                    continue;
                };
                if tx == u32::MAX || !inside {
                    continue;
                }
                let texel = sink.textures[&key].pixel(tx, ty).expect("texel").to_array();
                // The texel's place in its level.
                let rect = key.rect((c.area().width, c.area().height));
                let (lx, ly) = (rect.x + tx, rect.y + ty);
                let k = f64::from(1u32 << key.level);
                assert_eq!(
                    ((a.0 / k).floor() as u32, (a.1 / k).floor() as u32),
                    (lx, ly),
                    "{pane:?} at {p:?}: the texture is placed where its level's pixel is"
                );
                let want = c
                    .render_tile(
                        pane,
                        mode,
                        &colors,
                        TileRect {
                            x: lx,
                            y: ly,
                            width: 1,
                            height: 1,
                        },
                        key.level,
                    )
                    .pixel(0, 0)
                    .expect("pixel")
                    .to_array();
                assert_eq!(texel, want, "{pane:?} level {} at ({lx}, {ly})", key.level);
                let _ = painted(texel, p, anchor, cell, &colors);
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}

/// A13.12 at 100 %: the painted checkerboard equals `composite_checkerboard`'s cells of the level-0 render.
#[test]
fn a13_12_checkerboard_at_100_percent_equals_core() {
    let world = World::new();
    let mut m = view(&world, 300, 200);
    m.zoom_to(crate::picture::ZoomChoice::Actual);
    // The area's origin on a whole pixel.
    m.viewport.center = (150.0, 100.0);
    let mut sink = FakeSink::default();
    settle(&mut m, &world, &mut sink, TEXTURE_BUDGET);
    let c = Arc::clone(m.comparison().expect("comparison"));
    let colors = m.colors();
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        let mut want = c.render_tile(
            pane,
            m.shown_mode(),
            &colors,
            TileRect {
                x: 0,
                y: 0,
                width: 300,
                height: 200,
            },
            0,
        );
        composite_checkerboard(&mut want, (0, 0), &colors, 8);
        let list = m.draw_list(pane);
        let (anchor, cell) = m.checker_anchor(pane).expect("anchor");
        assert_eq!(cell, 8.0);
        let slot = m
            .slots()
            .iter()
            .find(|s| s.pane == pane)
            .copied()
            .expect("slot");
        let mut n = 0;
        for y in (0..200).step_by(7) {
            for x in (0..300).step_by(7) {
                let p = m.viewport.to_screen(
                    slot.picture,
                    1.0,
                    m.area(),
                    (f64::from(x) + 0.5, f64::from(y) + 0.5),
                );
                if !slot.picture.contains(p) {
                    continue;
                }
                let Some((key, tx, ty)) = texel_at(&list, p, &sink) else {
                    panic!("undrawn");
                };
                let texel = sink.textures[&key].pixel(tx, ty).expect("texel").to_array();
                assert_eq!(
                    painted(texel, p, anchor, cell, &colors),
                    want.pixel(x, y).expect("pixel").to_array(),
                    "{pane:?} ({x}, {y})"
                );
                n += 1;
            }
        }
        assert!(n > 100, "{pane:?}: {n}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("HIKASA_HEAVY").map_or(12, |_| 10_000),
        ..ProptestConfig::default()
    })]

    /// P-TILESCREEN over random geometries, modes, zooms and pans. Mutation: the eviction order newest first (a
    /// small budget evicts textures still drawn) and a tile key without the generation.
    #[test]
    fn p_tilescreen(
        offset in (-60i32..60, -40i32..40),
        flip in any::<bool>(),
        mode in 0u8..3,
        zoom in 0usize..25,
        pan in (-400.0f64..400.0, -300.0f64..300.0),
    ) {
        let world = World::new();
        let mut m = view(&world, 600, 420);
        m.options.geometry.offset = offset;
        if flip {
            m.options.geometry.orientation.right = hikasa_picture::Orientation::Cw90;
        }
        m.reload(&world.service);
        wait(&mut m, &world);
        m.set_mode(match mode {
            0 => DiffMode::Tolerance,
            1 => DiffMode::Range,
            _ => DiffMode::Blend { percent: 37 },
        });
        m.arrange(CONTENT, 0.0);
        m.viewport.zoom = Zoom::Scale(crate::picture::viewport::ZOOM_STEPS[zoom].max(1.0 / 8.0));
        m.viewport.center = (m.area().0 / 2.0, m.area().1 / 2.0);
        let panes: Vec<PaneRect> = m.slots().iter().map(|s| s.picture).collect();
        let area = m.area();
        m.viewport.pan_by(&panes, area, pan);
        tile_screen(&world, &mut m, 23);
    }

    /// A13.12's budget: random viewport scripts with a small budget never hold more than it and never evict a texture
    /// drawn in the frame; the evictions go least recently drawn first.
    #[test]
    fn p_budget(
        steps in proptest::collection::vec((0usize..25, -600.0f64..600.0, -400.0f64..400.0), 1..8),
    ) {
        let world = World::new();
        let mut m = view(&world, 900, 700);
        let budget = 3 << 20;
        let mut sink = FakeSink::default();
        for (zoom, dx, dy) in steps {
            m.viewport.zoom = Zoom::Scale(crate::picture::viewport::ZOOM_STEPS[zoom].max(1.0 / 4.0));
            let panes: Vec<PaneRect> = m.slots().iter().map(|s| s.picture).collect();
            let area = m.area();
            m.viewport.pan_by(&panes, area, (dx, dy));
            for round in 0..2 {
                let before = sink.evicted.len();
                let logged = m.tiles.evictions().len();
                m.frame(&world.service, budget, &mut sink);
                sink.end_frame();
                prop_assert!(m.tiles.bytes() <= budget, "{} > {budget}", m.tiles.bytes());
                prop_assert_eq!(m.tiles.bytes(), sink.bytes());
                // Nothing drawn this frame was evicted by it.
                let visible: std::collections::HashSet<TileKey> = m
                    .tiles
                    .wanted()
                    .iter()
                    .filter(|(_, v)| *v)
                    .map(|(k, _)| *k)
                    .collect();
                for k in &sink.evicted[before..] {
                    prop_assert!(!visible.contains(k), "{k:?} is drawn and was evicted");
                }
                // LRU: the budget's evictions of one frame go least recently drawn first.
                let burst: Vec<u64> = m.tiles.evictions()[logged..]
                    .iter()
                    .filter(|(k, _)| k.generation == m.generations[k.pane as usize])
                    .map(|(_, at)| *at)
                    .collect();
                prop_assert!(burst.windows(2).all(|w| w[0] <= w[1]), "{burst:?}");
                if round == 0 && let Some(source) = m.render_source() {
                    m.tiles.render_now(&world.service, &source);
                }
            }
        }
        prop_assert!(TILE == 256);
    }
}
