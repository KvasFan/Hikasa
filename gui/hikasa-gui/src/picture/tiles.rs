//! Tiles and textures of the Picture Compare view (PX-002, PX-007, UI-023; §1.13.11, D-363): the level of a scale,
//! tile keys by pane generation, the overview of each pane, the batch jobs that render the missing tiles on the
//! Service pool (`render_tile`, named only here in `gui/`), the uploads of a frame (≤ 8 tiles, ≤ 2 MiB, visible first,
//! nearest the pane centre first), the 64 MiB texture budget with LRU eviction of tiles not drawn, and the draw list
//! the painter follows. Toolkit-free: uploads and evictions go to a [`TextureSink`] — `ui/picture.rs` (the only owner
//! of textures) in the window, a recording fake in tests.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::{Receiver, Sender, unbounded};
use hikasa_core::JobKind;
use hikasa_picture::{DiffMode, Pane, PictureColors, PictureComparison, RgbaTile, TileRect};
use parking_lot::Mutex;

use super::viewport::{PaneRect, Viewport};
use crate::job::{Job, Wake};

/// The tile size of every level (texels).
pub const TILE: u32 = 256;
/// The resident texels of all picture views of a window (bytes, 4 a texel).
pub const TEXTURE_BUDGET: u64 = 64 << 20;
/// Uploads of one frame (**open default**).
pub const UPLOADS_PER_FRAME: usize = 8;
/// Bytes uploaded in one frame (**open default**; an overview alone may exceed it).
pub const UPLOAD_BYTES_PER_FRAME: u64 = 2 << 20;
/// The overview fits this many texels each way (≤ 4 MiB).
pub const OVERVIEW_MAX: u32 = 1024;
/// The largest level.
pub const MAX_LEVEL: u8 = 8;
/// Rows of an overview band rendered as one task.
const BAND: u32 = 32;

/// The three panes in their order (the index of a pane in `generations`, `shown`).
pub const PANES: [Pane; 3] = [Pane::Left, Pane::Right, Pane::Difference];

/// The index of a pane.
pub fn pane_index(pane: Pane) -> usize {
    match pane {
        Pane::Left => 0,
        Pane::Right => 1,
        Pane::Difference => 2,
    }
}

/// The level of a scale (normative): `k = clamp(⌊log₂(1/z)⌋, 0, 8)`, the coarsest level still at least as detailed
/// as the screen.
pub fn level_for(scale: f64) -> u8 {
    if scale.is_nan() || scale <= 0.0 {
        return MAX_LEVEL;
    }
    let k = (1.0 / scale).log2().floor();
    k.clamp(0.0, f64::from(MAX_LEVEL)) as u8
}

/// The size of the area at `level` (`⌈W / 2ᵏ⌉ × ⌈H / 2ᵏ⌉`, as `PictureComparison::level_size`).
pub fn level_size(area: (u32, u32), level: u8) -> (u32, u32) {
    let k = u32::from(level.min(MAX_LEVEL));
    let up = |n: u32| ((u64::from(n) + (1 << k) - 1) >> k) as u32;
    (up(area.0), up(area.1))
}

/// The overview's level: the finest whose size fits [`OVERVIEW_MAX`] each way; `None` for an area so large that even
/// level 8 does not (an offset near ±2²⁰: the tiles alone draw it).
pub fn overview_level(area: (u32, u32)) -> Option<u8> {
    (0..=MAX_LEVEL).find(|&k| {
        let (w, h) = level_size(area, k);
        w <= OVERVIEW_MAX && h <= OVERVIEW_MAX
    })
}

/// A texture of a pane: a tile of a level's grid, or the pane's overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// The pane's index (0 left, 1 right, 2 difference).
    pub pane: u8,
    /// The pane's generation when it was wanted (§1.13.9's table).
    pub generation: u64,
    /// The level.
    pub level: u8,
    /// Tile column in the level's grid (0 for the overview).
    pub x: u32,
    /// Tile row.
    pub y: u32,
    /// The overview (the whole level in one texture).
    pub overview: bool,
}

impl TileKey {
    /// The level rectangle the key covers.
    pub fn rect(&self, area: (u32, u32)) -> TileRect {
        let (w, h) = level_size(area, self.level);
        if self.overview {
            return TileRect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            };
        }
        let (x, y) = (self.x * TILE, self.y * TILE);
        TileRect {
            x,
            y,
            width: TILE.min(w.saturating_sub(x)),
            height: TILE.min(h.saturating_sub(y)),
        }
    }
}

/// Where a texture was rendered: the area's origin in left-picture coordinates and the right picture's offset of
/// that comparison (a right texture is drawn shifted by the offset not landed yet, §1.13.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Basis {
    /// The area's top-left corner in left coordinates.
    pub origin: (i64, i64),
    /// The right picture's offset.
    pub offset: (i32, i32),
    /// The area's size.
    pub area: (u32, u32),
}

/// The receiver of uploads and evictions (the window's texture owner, or a test's recorder).
pub trait TextureSink {
    /// Uploads a texture under `key`.
    fn upload(&mut self, key: TileKey, image: &RgbaTile);
    /// Drops the texture of `key`.
    fn evict(&mut self, key: TileKey);
}

/// An uploaded texture.
#[derive(Debug, Clone, Copy)]
struct Resident {
    basis: Basis,
    bytes: u64,
    last_drawn: u64,
}

/// A rendered texture handed over by a batch job.
struct Landed {
    key: TileKey,
    basis: Basis,
    image: Arc<RgbaTile>,
}

/// One texture drawn: its key, its destination in screen points `(x0, y0, x1, y1)` and its texture coordinates
/// `(u0, v0, u1, v1)` (0…1). Older generations come first (under), then the overview, then tiles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawItem {
    /// The texture.
    pub key: TileKey,
    /// Destination (points).
    pub rect: (f64, f64, f64, f64),
    /// Texture coordinates.
    pub uv: (f64, f64, f64, f64),
}

/// What a frame of a pane needs to know.
#[derive(Debug, Clone, Copy)]
pub struct PaneFrame {
    /// The pane.
    pub pane: Pane,
    /// Its rectangle (points).
    pub rect: PaneRect,
}

/// The view's state of a frame: the viewport, the landed comparison's basis and the offset wanted.
#[derive(Debug, Clone, Copy)]
pub struct FrameInput {
    /// The viewport.
    pub viewport: Viewport,
    /// The scale (Fit resolved).
    pub scale: f64,
    /// The landed comparison's basis.
    pub basis: Basis,
    /// The offset wanted (a drag's latest).
    pub target_offset: (i32, i32),
    /// The panes' generations.
    pub generations: [u64; 3],
    /// The frame's number (LRU).
    pub frame: u64,
    /// The bytes this view may hold (the window's budget minus the other views').
    pub budget: u64,
    /// The focused pane's index: its overview is wanted first.
    pub focus: u8,
}

/// What a batch job renders with.
#[derive(Clone)]
pub struct RenderSource {
    /// The comparison.
    pub comparison: Arc<PictureComparison>,
    /// The difference pane's mode.
    pub mode: DiffMode,
    /// The colours.
    pub colors: PictureColors,
    /// Its basis.
    pub basis: Basis,
}

/// Counters of the tile work (tests, the bench).
#[derive(Debug, Default)]
pub struct TileCounters {
    /// Batch jobs started.
    pub batches: AtomicUsize,
    /// Textures rendered.
    pub rendered: AtomicUsize,
    /// Textures skipped because no longer wanted.
    pub skipped: AtomicUsize,
}

/// The tiles of a view.
pub struct TileState {
    resident: HashMap<TileKey, Resident>,
    ready: HashMap<TileKey, (Basis, Arc<RgbaTile>)>,
    pending: HashSet<TileKey>,
    wanted: Arc<Mutex<HashSet<TileKey>>>,
    /// The wanted keys of the last frame in the order they are wanted (overviews, visible nearest first, ring).
    order: Vec<(TileKey, bool)>,
    tx: Sender<Landed>,
    rx: Receiver<Landed>,
    jobs: Vec<Job<()>>,
    bytes: u64,
    /// The level shift that keeps the visible tiles within the budget.
    bias: u8,
    /// Uploads of the last frame (tests).
    last_uploads: Vec<TileKey>,
    /// Every eviction with the texture's last drawn frame (tests: LRU order).
    evictions: Vec<(TileKey, u64)>,
    /// The counters.
    pub counters: Arc<TileCounters>,
    /// The tile jobs run (a test turns this off to drive renders by hand).
    pub(crate) run_jobs: bool,
}

impl Default for TileState {
    fn default() -> TileState {
        let (tx, rx) = unbounded();
        TileState {
            resident: HashMap::new(),
            ready: HashMap::new(),
            pending: HashSet::new(),
            wanted: Arc::new(Mutex::new(HashSet::new())),
            order: Vec::new(),
            tx,
            rx,
            jobs: Vec::new(),
            bytes: 0,
            bias: 0,
            last_uploads: Vec::new(),
            evictions: Vec::new(),
            counters: Arc::new(TileCounters::default()),
            run_jobs: true,
        }
    }
}

/// The visible tile range of a pane at `level`: `(x0, y0, x1, y1)` tile indexes, end exclusive, with `ring` extra
/// tiles around, clipped to the level's grid. `None` when the pane shows no part of the area.
fn tile_range(
    input: &FrameInput,
    pane: PaneRect,
    level: u8,
    ring: u32,
) -> Option<(u32, u32, u32, u32)> {
    let area = input.basis.area;
    let (lw, lh) = level_size(area, level);
    if lw == 0 || lh == 0 {
        return None;
    }
    let areaf = (f64::from(area.0), f64::from(area.1));
    let (x0, y0, x1, y1) = input.viewport.visible(pane, input.scale, areaf);
    let s = f64::from(1u32 << level);
    let t = f64::from(TILE) * s;
    let cols = lw.div_ceil(TILE);
    let rows = lh.div_ceil(TILE);
    let lo = |v: f64, n: u32| -> u32 { ((v / t).floor().max(0.0) as u32).min(n) };
    let hi = |v: f64, n: u32| -> u32 { ((v / t).ceil().max(0.0) as u32).min(n) };
    let (tx0, ty0, tx1, ty1) = (lo(x0, cols), lo(y0, rows), hi(x1, cols), hi(y1, rows));
    if tx0 >= tx1 || ty0 >= ty1 {
        return None;
    }
    Some((
        tx0.saturating_sub(ring),
        ty0.saturating_sub(ring),
        (tx1 + ring).min(cols),
        (ty1 + ring).min(rows),
    ))
}

/// Bytes of a level rectangle.
fn rect_bytes(r: TileRect) -> u64 {
    u64::from(r.width) * u64::from(r.height) * 4
}

impl TileState {
    /// The resident bytes (the budget's count; A13.12's and the bench's figure).
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Every eviction so far with the texture's last drawn frame.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn evictions(&self) -> &[(TileKey, u64)] {
        &self.evictions
    }

    /// True when the last frame wanted nothing (no comparison or no pane on screen yet).
    pub fn wanted_is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// True when every visible texture the last frame wanted is uploaded (the panes are drawn at their level).
    pub fn complete(&self) -> bool {
        self.order
            .iter()
            .filter(|(_, visible)| *visible)
            .all(|(k, _)| self.resident.contains_key(k))
    }

    /// True when the visible textures of pane `index` the last frame wanted are uploaded.
    pub fn pane_complete(&self, index: u8) -> bool {
        self.order
            .iter()
            .filter(|(k, visible)| *visible && k.pane == index)
            .all(|(k, _)| self.resident.contains_key(k))
    }

    /// True while a render or an upload is pending (the window keeps repainting).
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.ready.is_empty() || !self.rx.is_empty()
    }

    /// The wanted keys of the last frame (visible ones flagged).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wanted(&self) -> &[(TileKey, bool)] {
        &self.order
    }

    /// The level a pane draws at for `scale` (the budget's shift applied).
    pub fn level_of(&self, scale: f64) -> u8 {
        (level_for(scale) + self.bias).min(MAX_LEVEL)
    }

    /// The keys a pane wants at `level` (overview first, then the visible tiles nearest the centre first, then the
    /// ring), each with whether it is visible. When the overview's level is at or finer than `level`, the overview
    /// serves as the visible tiles (no tile of a coarser level is rendered).
    fn pane_wants(
        &self,
        input: &FrameInput,
        frame: &PaneFrame,
        level: u8,
        out: &mut Vec<(TileKey, bool)>,
    ) {
        let index = pane_index(frame.pane);
        let generation = input.generations[index];
        let overview = overview_level(input.basis.area);
        if let Some(ol) = overview {
            out.push((
                TileKey {
                    pane: index as u8,
                    generation,
                    level: ol,
                    x: 0,
                    y: 0,
                    overview: true,
                },
                true,
            ));
            if ol <= level {
                return;
            }
        }
        let Some((x0, y0, x1, y1)) = tile_range(input, frame.rect, level, 0) else {
            return;
        };
        let key = |x: u32, y: u32| TileKey {
            pane: index as u8,
            generation,
            level,
            x,
            y,
            overview: false,
        };
        // Nearest the pane's centre first.
        let areaf = (f64::from(input.basis.area.0), f64::from(input.basis.area.1));
        let c = input
            .viewport
            .to_area(frame.rect, input.scale, areaf, frame.rect.center());
        let t = f64::from(TILE) * f64::from(1u32 << level);
        let mut visible: Vec<(f64, TileKey)> = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let (mx, my) = ((f64::from(x) + 0.5) * t, (f64::from(y) + 0.5) * t);
                visible.push(((mx - c.0).powi(2) + (my - c.1).powi(2), key(x, y)));
            }
        }
        visible.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.extend(visible.into_iter().map(|(_, k)| (k, true)));
        if let Some((rx0, ry0, rx1, ry1)) = tile_range(input, frame.rect, level, 1) {
            for y in ry0..ry1 {
                for x in rx0..rx1 {
                    if !(x0..x1).contains(&x) || !(y0..y1).contains(&y) {
                        out.push((key(x, y), false));
                    }
                }
            }
        }
    }

    /// One frame: hands landed renders over, decides the wanted keys (the budget's level shift included), starts a
    /// batch job for the missing ones, uploads at most [`UPLOADS_PER_FRAME`] (visible first, nearest the centre
    /// first), evicts what the budget needs (tiles not drawn this frame, least recently drawn first) and the older
    /// generations a pane's new overview replaced. Returns true when something changed (a repaint).
    pub fn frame(
        &mut self,
        service: &hikasa_session::Service,
        wake: &Wake,
        input: &FrameInput,
        panes: &[PaneFrame],
        source: Option<&RenderSource>,
        sink: &mut dyn TextureSink,
    ) -> bool {
        let mut changed = self.drain();
        self.jobs.retain(|j| j.try_result().is_none());
        // The wanted set, one level coarser while the visible tiles would not fit.
        let base = level_for(input.scale);
        let mut bias = 0u8;
        let mut order = Vec::new();
        loop {
            order.clear();
            let level = (base + bias).min(MAX_LEVEL);
            // Every pane's overview first (each pane's first screen), then the visible tiles of all panes nearest
            // their centres first (ranks interleaved), then the rings.
            let mut per_pane = Vec::new();
            for frame in panes {
                let mut wants = Vec::new();
                self.pane_wants(input, frame, level, &mut wants);
                per_pane.push(wants);
            }
            let mut overviews: Vec<(TileKey, bool)> = per_pane
                .iter()
                .flatten()
                .filter(|(k, _)| k.overview)
                .copied()
                .collect();
            overviews.sort_by_key(|(k, _)| k.pane != input.focus);
            order.extend(overviews);
            let visible: Vec<Vec<(TileKey, bool)>> = per_pane
                .iter()
                .map(|w| {
                    w.iter()
                        .filter(|(k, v)| *v && !k.overview)
                        .copied()
                        .collect()
                })
                .collect();
            let longest = visible.iter().map(Vec::len).max().unwrap_or(0);
            for rank in 0..longest {
                order.extend(visible.iter().filter_map(|w| w.get(rank)).copied());
            }
            order.extend(per_pane.iter().flatten().filter(|(_, v)| !*v).copied());
            let bytes: u64 = order
                .iter()
                .filter(|(_, visible)| *visible)
                .map(|(k, _)| rect_bytes(k.rect(input.basis.area)))
                .sum();
            if bytes <= input.budget || base + bias >= MAX_LEVEL {
                // The rings are prefetch: when the visible set and the rings would not fit, there is no ring (a ring
                // tile would evict another wanted one, which would be rendered again: no churn).
                let all: u64 = order
                    .iter()
                    .map(|(k, _)| rect_bytes(k.rect(input.basis.area)))
                    .sum();
                if all > input.budget {
                    order.retain(|(_, visible)| *visible);
                }
                break;
            }
            bias += 1;
        }
        self.bias = bias;
        {
            let mut wanted = self.wanted.lock();
            wanted.clear();
            wanted.extend(order.iter().map(|(k, _)| *k));
        }
        // Renders no longer wanted are dropped.
        let wanted_now: HashSet<TileKey> = order.iter().map(|(k, _)| *k).collect();
        self.ready.retain(|k, _| wanted_now.contains(k));
        // A batch skips a key no longer wanted when its turn comes: it is no longer pending either (it is wanted
        // again, and asked again, only in a later frame, which sees it missing).
        self.pending.retain(|k| wanted_now.contains(k));
        // A batch for the keys nobody has.
        let missing: Vec<TileKey> = order
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| {
                !self.resident.contains_key(k)
                    && !self.ready.contains_key(k)
                    && !self.pending.contains(k)
            })
            .collect();
        if !missing.is_empty()
            && let Some(source) = source
        {
            self.pending.extend(missing.iter().copied());
            if self.run_jobs {
                self.start_batch(service, wake, source.clone(), missing);
            }
            changed = true;
        }
        // Mark what is used this frame: the visible keys are drawn (never evicted now), the rings' are kept as recent.
        for (k, _) in &order {
            if let Some(r) = self.resident.get_mut(k) {
                r.last_drawn = input.frame;
            }
        }
        let visible_now: HashSet<TileKey> = order
            .iter()
            .filter(|(_, visible)| *visible)
            .map(|(k, _)| *k)
            .collect();
        // Uploads.
        self.last_uploads.clear();
        let mut sent = 0u64;
        for (key, _) in &order {
            if self.last_uploads.len() >= UPLOADS_PER_FRAME {
                break;
            }
            let Some((basis, image)) = self.ready.get(key).cloned() else {
                continue;
            };
            let bytes = u64::from(image.width) * u64::from(image.height) * 4;
            if !self.last_uploads.is_empty() && sent + bytes > UPLOAD_BYTES_PER_FRAME {
                break;
            }
            if !self.make_room(bytes, input, &visible_now, sink) {
                continue;
            }
            self.ready.remove(key);
            sink.upload(*key, &image);
            self.resident.insert(
                *key,
                Resident {
                    basis,
                    bytes,
                    last_drawn: input.frame,
                },
            );
            self.bytes += bytes;
            sent += bytes;
            self.last_uploads.push(*key);
            changed = true;
        }
        // A pane whose current overview is resident drops its older generations.
        for frame in panes {
            let index = pane_index(frame.pane) as u8;
            let generation = input.generations[index as usize];
            let has_overview = self
                .resident
                .keys()
                .any(|k| k.pane == index && k.overview && k.generation == generation);
            let covered = has_overview || overview_level(input.basis.area).is_none();
            if covered {
                let old: Vec<TileKey> = self
                    .resident
                    .keys()
                    .filter(|k| k.pane == index && k.generation < generation)
                    .copied()
                    .collect();
                for k in old {
                    self.evict(k, sink);
                    changed = true;
                }
            }
        }
        self.order = order;
        changed
    }

    /// Evicts tiles not drawn in this frame (least recently drawn first) until `bytes` more fit the budget; false
    /// when they do not (every resident texture is drawn).
    fn make_room(
        &mut self,
        bytes: u64,
        input: &FrameInput,
        visible: &HashSet<TileKey>,
        sink: &mut dyn TextureSink,
    ) -> bool {
        if self.bytes + bytes <= input.budget {
            return true;
        }
        let mut candidates: Vec<(u64, TileKey)> = self
            .resident
            .iter()
            .filter(|(k, _)| !visible.contains(k))
            .map(|(k, r)| (r.last_drawn, *k))
            .collect();
        candidates.sort();
        for (_, key) in candidates {
            if self.bytes + bytes <= input.budget {
                break;
            }
            self.evict(key, sink);
        }
        self.bytes + bytes <= input.budget
    }

    fn evict(&mut self, key: TileKey, sink: &mut dyn TextureSink) {
        if let Some(r) = self.resident.remove(&key) {
            self.bytes -= r.bytes;
            self.evictions.push((key, r.last_drawn));
            sink.evict(key);
        }
    }

    /// Takes what the batch jobs landed.
    fn drain(&mut self) -> bool {
        let mut any = false;
        while let Ok(landed) = self.rx.try_recv() {
            self.pending.remove(&landed.key);
            if self.wanted.lock().contains(&landed.key) {
                self.ready.insert(landed.key, (landed.basis, landed.image));
                any = true;
            }
        }
        any
    }

    /// A tab no longer shown drops its tiles and keeps its overviews (§1.13.11).
    pub fn drop_tiles(&mut self, sink: &mut dyn TextureSink) {
        let tiles: Vec<TileKey> = self
            .resident
            .keys()
            .filter(|k| !k.overview)
            .copied()
            .collect();
        for k in tiles {
            self.evict(k, sink);
        }
        self.ready.retain(|k, _| k.overview);
    }

    /// Cancels the batch jobs (the view closes).
    pub fn cancel(&self) {
        for job in &self.jobs {
            job.cancel();
        }
    }

    /// Starts a batch job: renders `keys` in order on the Service pool (an overview in bands of rows in parallel),
    /// skipping a key no longer wanted when its turn comes, and hands each over as it completes.
    fn start_batch(
        &mut self,
        service: &hikasa_session::Service,
        wake: &Wake,
        source: RenderSource,
        keys: Vec<TileKey>,
    ) {
        self.counters.batches.fetch_add(1, Ordering::Relaxed);
        let wanted = Arc::clone(&self.wanted);
        let tx = self.tx.clone();
        let counters = Arc::clone(&self.counters);
        let notify = Arc::clone(wake);
        let title = format!("{} tiles", keys.len());
        let job = Job::spawn(
            service,
            JobKind::PictureCompare,
            title,
            wake,
            move |svc, job| {
                render_batch(
                    svc,
                    &source,
                    &keys,
                    &wanted,
                    &counters,
                    &|key, image| {
                        let _ = tx.send(Landed {
                            key,
                            basis: source.basis,
                            image: Arc::new(image),
                        });
                        notify();
                    },
                    &|| job.cancel.is_cancelled(),
                );
                Ok(())
            },
        );
        self.jobs.push(job);
    }

    /// Renders and hands over `keys` now, on this thread (tests drive renders without jobs).
    #[cfg(test)]
    pub(crate) fn render_now(&mut self, service: &hikasa_session::Service, source: &RenderSource) {
        let keys: Vec<TileKey> = self.pending.iter().copied().collect();
        let tx = self.tx.clone();
        render_batch(
            service,
            source,
            &keys,
            &self.wanted,
            &self.counters,
            &|key, image| {
                let _ = tx.send(Landed {
                    key,
                    basis: source.basis,
                    image: Arc::new(image),
                });
            },
            &|| false,
        );
    }

    /// The draw list of a pane: for each generation present (older under newer), the overview then the tiles of the
    /// level drawn, each mapped through its basis (a right texture shifted by the offset not landed yet); an older
    /// generation's texture is clipped away where a resident tile of the current generation covers it.
    pub fn draw_list(&self, input: &FrameInput, frame: &PaneFrame) -> Vec<DrawItem> {
        let index = pane_index(frame.pane) as u8;
        let generation = input.generations[index as usize];
        let mut keys: Vec<(&TileKey, &Resident)> = self
            .resident
            .iter()
            .filter(|(k, _)| k.pane == index && k.generation <= generation)
            .collect();
        // Older generations first; per generation the overview, then coarser levels, then finer.
        keys.sort_by(|a, b| {
            a.0.generation
                .cmp(&b.0.generation)
                .then(b.0.overview.cmp(&a.0.overview))
                .then(b.0.level.cmp(&a.0.level))
                .then(a.0.cmp(b.0))
        });
        let current_tiles: Vec<(f64, f64, f64, f64)> = keys
            .iter()
            .filter(|(k, _)| k.generation == generation && !k.overview)
            .map(|(k, r)| self.screen_rect(input, frame, k, r))
            .collect();
        let shown_level = self.level_of(input.scale);
        let mut out = Vec::new();
        // The difference pane before its new overview arrives (a geometry change): the right picture's overviews,
        // shifted to the offset wanted, stand in where no older difference texture lies (no undrawn frame).
        if index == 2
            && !self
                .resident
                .keys()
                .any(|k| k.pane == 2 && k.overview && k.generation == generation)
        {
            let mut right: Vec<(&TileKey, &Resident)> = self
                .resident
                .iter()
                .filter(|(k, _)| k.pane == 1 && k.overview)
                .collect();
            right.sort_by_key(|(k, _)| k.generation);
            for (k, r) in right {
                out.push(DrawItem {
                    key: *k,
                    rect: self.screen_rect(input, frame, k, r),
                    uv: (0.0, 0.0, 1.0, 1.0),
                });
            }
        }
        for (k, r) in keys {
            // Tiles of another level than the one drawn now are not drawn (their overview is under them).
            if !k.overview && k.generation == generation && k.level != shown_level {
                continue;
            }
            let rect = self.screen_rect(input, frame, k, r);
            let full = DrawItem {
                key: *k,
                rect,
                uv: (0.0, 0.0, 1.0, 1.0),
            };
            if k.generation == generation {
                out.push(full);
            } else {
                clip_out(full, &current_tiles, &mut out);
            }
        }
        out
    }

    fn screen_rect(
        &self,
        input: &FrameInput,
        frame: &PaneFrame,
        key: &TileKey,
        r: &Resident,
    ) -> (f64, f64, f64, f64) {
        let tr = key.rect(r.basis.area);
        let s = f64::from(1u32 << key.level);
        let shift = if key.pane == 1 {
            (
                f64::from(input.target_offset.0) - f64::from(r.basis.offset.0),
                f64::from(input.target_offset.1) - f64::from(r.basis.offset.1),
            )
        } else {
            (0.0, 0.0)
        };
        // Area pixels of the texture in the landed area's coordinates.
        let ox = (r.basis.origin.0 - input.basis.origin.0) as f64 + shift.0;
        let oy = (r.basis.origin.1 - input.basis.origin.1) as f64 + shift.1;
        let x0 = ox + f64::from(tr.x) * s;
        let y0 = oy + f64::from(tr.y) * s;
        let x1 = x0 + f64::from(tr.width) * s;
        let y1 = y0 + f64::from(tr.height) * s;
        // The level's last texel may extend past the area (⌈W/2ᵏ⌉): clip it to what the texel covers.
        let areaf = (f64::from(input.basis.area.0), f64::from(input.basis.area.1));
        let a = input
            .viewport
            .to_screen(frame.rect, input.scale, areaf, (x0, y0));
        let b = input
            .viewport
            .to_screen(frame.rect, input.scale, areaf, (x1, y1));
        (a.0, a.1, b.0, b.1)
    }
}

/// Emits the parts of `item` outside every rectangle of `cover` (each part with its texture coordinates).
fn clip_out(item: DrawItem, cover: &[(f64, f64, f64, f64)], out: &mut Vec<DrawItem>) {
    let mut parts = vec![item];
    for c in cover {
        let mut next = Vec::new();
        for p in parts {
            let (x0, y0, x1, y1) = p.rect;
            if c.2 <= x0 || c.0 >= x1 || c.3 <= y0 || c.1 >= y1 {
                next.push(p);
                continue;
            }
            let sub = |nx0: f64, ny0: f64, nx1: f64, ny1: f64| {
                let fx = |x: f64| p.uv.0 + (x - x0) / (x1 - x0) * (p.uv.2 - p.uv.0);
                let fy = |y: f64| p.uv.1 + (y - y0) / (y1 - y0) * (p.uv.3 - p.uv.1);
                DrawItem {
                    key: p.key,
                    rect: (nx0, ny0, nx1, ny1),
                    uv: (fx(nx0), fy(ny0), fx(nx1), fy(ny1)),
                }
            };
            // Above, below, left, right of the cover.
            if c.1 > y0 {
                next.push(sub(x0, y0, x1, c.1));
            }
            if c.3 < y1 {
                next.push(sub(x0, c.3, x1, y1));
            }
            let (my0, my1) = (c.1.max(y0), c.3.min(y1));
            if c.0 > x0 {
                next.push(sub(x0, my0, c.0, my1));
            }
            if c.2 < x1 {
                next.push(sub(c.2, my0, x1, my1));
            }
        }
        parts = next;
    }
    out.extend(
        parts
            .into_iter()
            .filter(|p| p.rect.2 > p.rect.0 && p.rect.3 > p.rect.1),
    );
}

/// Renders `keys` with `source`: overviews in bands of rows in parallel on the Service pool, tiles one a task;
/// a key no longer `wanted` when its turn comes is skipped; each finished key goes to `deliver`.
fn render_batch(
    service: &hikasa_session::Service,
    source: &RenderSource,
    keys: &[TileKey],
    wanted: &Mutex<HashSet<TileKey>>,
    counters: &TileCounters,
    deliver: &(dyn Fn(TileKey, RgbaTile) + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
) {
    let pane = |k: &TileKey| PANES[usize::from(k.pane).min(2)];
    let area = source.basis.area;
    let next = AtomicUsize::new(0);
    let tiles: Vec<TileKey> = keys.iter().filter(|k| !k.overview).copied().collect();
    for key in keys.iter().filter(|k| k.overview) {
        if cancelled() || !wanted.lock().contains(key) {
            counters.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let rect = key.rect(area);
        let bands = rect.height.div_ceil(BAND).max(1);
        let parts: Vec<Mutex<Option<RgbaTile>>> = (0..bands).map(|_| Mutex::new(None)).collect();
        let band_next = AtomicUsize::new(0);
        let threads = service.pool().current_num_threads().max(1);
        service.pool().scope(|s| {
            for _ in 0..threads {
                s.spawn(|_| {
                    loop {
                        let b = band_next.fetch_add(1, Ordering::Relaxed) as u32;
                        // A newer generation (a drag) makes this overview unwanted: its bands stop at once.
                        if b >= bands || cancelled() || !wanted.lock().contains(key) {
                            break;
                        }
                        let y = b * BAND;
                        let band = TileRect {
                            x: 0,
                            y,
                            width: rect.width,
                            height: BAND.min(rect.height - y),
                        };
                        let image = source.comparison.render_tile(
                            pane(key),
                            source.mode,
                            &source.colors,
                            band,
                            key.level,
                        );
                        *parts[b as usize].lock() = Some(image);
                    }
                });
            }
        });
        if cancelled() {
            return;
        }
        if !wanted.lock().contains(key) {
            counters.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut rgba = Vec::with_capacity(rect_bytes(rect) as usize);
        for part in parts {
            if let Some(image) = part.into_inner() {
                rgba.extend_from_slice(&image.rgba);
            }
        }
        counters.rendered.fetch_add(1, Ordering::Relaxed);
        deliver(
            *key,
            RgbaTile {
                width: rect.width,
                height: rect.height,
                rgba,
            },
        );
    }
    let threads = service.pool().current_num_threads().max(1);
    service.pool().scope(|s| {
        for _ in 0..threads.min(tiles.len().max(1)) {
            s.spawn(|_| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(key) = tiles.get(i) else {
                        break;
                    };
                    if cancelled() || !wanted.lock().contains(key) {
                        counters.skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let image = source.comparison.render_tile(
                        pane(key),
                        source.mode,
                        &source.colors,
                        key.rect(area),
                        key.level,
                    );
                    counters.rendered.fetch_add(1, Ordering::Relaxed);
                    deliver(*key, image);
                }
            });
        }
    });
}
