//! The hex view's scroll model and row cache (M2 W13a, contract §1.13.3, D-355), toolkit-free.
//!
//! The screen scrolls by row index (`u64`) and the part of the top row scrolled off, never by `f32` points (a 4 GiB pair
//! at 16 bytes a row has 2²⁸ rows). Each frame the window asks for the rows `[top, top + rows_visible + 1)` of the
//! current generation: cached rows are drawn; else `HexView::try_rows` (cached pages only, never a read) is asked for the
//! screen with a margin of one screen above and below; a miss draws placeholders and starts one prefetch job (`rows` in
//! the job, latest wins). This file is the only one of `gui/` that names `HexView::rows` and `HexView::try_rows`
//! (architecture check 7).

use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_hex::{DisplayFilter, FilterCache, HexAlignment, HexLayout, HexRow, HexView};
use hikasa_session::Service;
use hikasa_session::api::HexOpened;

use crate::job::{Job, Wake};

/// The most rows asked of `try_rows` and of a prefetch (three screens of at most 256 rows).
pub const MAX_ASK: u32 = 3 * 256;

/// What the rows are cut from: the opened documents, the landed alignment (`None`: None's provisional layout), the
/// layout, the filter and its context, and the filter's cache (kept per generation).
#[derive(Clone)]
pub struct Source {
    /// The opened documents.
    pub opened: Arc<HexOpened>,
    /// The landed alignment (`None`: provisional rows).
    pub alignment: Option<Arc<HexAlignment>>,
    /// The layout.
    pub layout: HexLayout,
    /// The display filter.
    pub filter: DisplayFilter,
    /// Context rows of `differences`.
    pub context: u8,
    /// The rows the filter shows, computed once per generation.
    pub cache: FilterCache,
}

impl Source {
    /// A source with a fresh filter cache.
    pub fn new(
        opened: Arc<HexOpened>,
        alignment: Option<Arc<HexAlignment>>,
        layout: HexLayout,
        filter: DisplayFilter,
        context: u8,
    ) -> Source {
        Source {
            opened,
            alignment,
            layout,
            filter,
            context,
            cache: FilterCache::default(),
        }
    }

    /// The view of the rows (cheap: the filter's rows are computed once, in the cache).
    pub fn view(&self) -> HexView<'_> {
        let (l, r) = (&*self.opened.documents.left, &*self.opened.documents.right);
        let view = match &self.alignment {
            Some(al) => HexView::new(l, r, al, self.layout),
            None => HexView::provisional(l, r, self.layout),
        };
        view.with_filter(self.filter, self.context)
            .reusing(&self.cache)
    }
}

/// Rows `[first, first + rows.len())` of a generation.
#[derive(Debug, Clone)]
struct RowSlice {
    generation: u64,
    first: u64,
    rows: Vec<HexRow>,
}

/// A landed prefetch.
struct Prefetched {
    generation: u64,
    first: u64,
    rows: Result<Vec<HexRow>, String>,
}

/// What the window draws for one row of the screen.
#[derive(Debug, Clone, PartialEq)]
pub enum ShownRow<'a> {
    /// The row.
    Row(&'a HexRow),
    /// Its pages are being read: the gap band, no address, no text.
    Placeholder,
    /// The rows could not be read (`Truncated` is an error, never zero bytes).
    Error(&'a str),
}

/// The screen of a hex view.
#[derive(Default)]
pub struct Screen {
    /// The first shown row (filtered index).
    pub top: u64,
    /// The part of the top row scrolled off (0…1).
    pub frac: f32,
    /// Whole rows the panes show.
    pub rows_visible: u32,
    cache: Option<RowSlice>,
    /// The prefetch asked last: first row, count, generation.
    pending: Option<(u64, u32, u64)>,
    prefetch: Option<Job<Prefetched>>,
    error: Option<(u64, String)>,
    /// Frames that drew a placeholder row (the benchmark's and the tests' counter).
    pub placeholder_frames: u64,
    /// Prefetch jobs started (tests).
    pub prefetches: u64,
}

impl Screen {
    /// The rows a frame needs: `[top, top + rows_visible + 1)`.
    fn wanted(&self, row_count: u64) -> (u64, u64) {
        let end = self
            .top
            .saturating_add(u64::from(self.rows_visible) + 1)
            .min(row_count);
        (self.top.min(end), end)
    }

    fn covers(&self, generation: u64, first: u64, end: u64) -> bool {
        self.cache.as_ref().is_some_and(|c| {
            c.generation == generation && c.first <= first && end <= c.first + c.rows.len() as u64
        })
    }

    /// Makes the rows of this frame available (cache, `try_rows`, or a prefetch job): call once per frame before
    /// drawing. True when every row of the frame is drawn from the cache.
    pub fn prepare(
        &mut self,
        src: &Source,
        generation: u64,
        service: &Service,
        wake: &Wake,
    ) -> bool {
        let view = src.view();
        let row_count = view.row_count();
        self.clamp(row_count);
        let (first, end) = self.wanted(row_count);
        if first >= end || self.covers(generation, first, end) {
            return true;
        }
        if self.error.as_ref().is_some_and(|(g, _)| *g == generation) {
            return false;
        }
        let page = u64::from(self.rows_visible.max(1));
        let ask_first = first.saturating_sub(page);
        let ask = (end - ask_first + page).min(u64::from(MAX_ASK)) as u32;
        match view.try_rows(ask_first, ask) {
            Some(Ok(rows)) => {
                self.cache = Some(RowSlice {
                    generation,
                    first: ask_first,
                    rows,
                });
                // A cached screen ends a pending prefetch's use (its answer still warms the caches).
                self.pending = None;
                true
            }
            Some(Err(e)) => {
                self.error = Some((generation, e.to_string()));
                false
            }
            None => {
                if self.pending != Some((ask_first, ask, generation)) {
                    self.start_prefetch(src, generation, ask_first, ask, service, wake);
                }
                self.placeholder_frames += 1;
                false
            }
        }
    }

    fn start_prefetch(
        &mut self,
        src: &Source,
        generation: u64,
        first: u64,
        count: u32,
        service: &Service,
        wake: &Wake,
    ) {
        // Latest wins: an older prefetch is cancelled (its pages stay in the caches it filled).
        if let Some(job) = self.prefetch.take() {
            job.cancel();
        }
        self.pending = Some((first, count, generation));
        self.prefetches += 1;
        let src = src.clone();
        self.prefetch = Some(Job::spawn(
            service,
            JobKind::HexCompare,
            "read the rows".into(),
            wake,
            move |_, _| {
                let rows = src.view().rows(first, count).map_err(|e| e.to_string());
                Ok(Prefetched {
                    generation,
                    first,
                    rows,
                })
            },
        ));
    }

    /// Polls the prefetch: a landed one of this generation and the pending range fills the cache; another only warmed
    /// the page caches. True when the screen changed.
    pub fn poll(&mut self, generation: u64) -> bool {
        let Some(result) = self.prefetch.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.prefetch = None;
        let Ok(landed) = result else {
            return false;
        };
        let current = self
            .pending
            .is_some_and(|(f, _, g)| f == landed.first && g == generation)
            && landed.generation == generation;
        self.pending = None;
        if !current {
            return true;
        }
        match landed.rows {
            Ok(rows) => {
                self.cache = Some(RowSlice {
                    generation,
                    first: landed.first,
                    rows,
                });
            }
            Err(e) => self.error = Some((generation, e)),
        }
        true
    }

    /// The row shown at filtered index `k` this frame.
    pub fn row(&self, generation: u64, k: u64) -> ShownRow<'_> {
        if let Some((g, e)) = &self.error
            && *g == generation
        {
            return ShownRow::Error(e);
        }
        match &self.cache {
            Some(c) if c.generation == generation && k >= c.first => c
                .rows
                .get((k - c.first) as usize)
                .map_or(ShownRow::Placeholder, ShownRow::Row),
            _ => ShownRow::Placeholder,
        }
    }

    /// The cached row at `k` of `generation`, if any.
    pub fn cached(&self, generation: u64, k: u64) -> Option<&HexRow> {
        match self.row(generation, k) {
            ShownRow::Row(r) => Some(r),
            _ => None,
        }
    }

    /// A prefetch runs.
    pub fn busy(&self) -> bool {
        self.prefetch.is_some()
    }

    /// The prefetch ended but its result is not published yet.
    pub fn settling(&self) -> bool {
        self.prefetch.as_ref().is_some_and(Job::settling)
    }

    /// Cancels the prefetch (the view closes).
    pub fn cancel(&mut self) {
        if let Some(job) = self.prefetch.take() {
            job.cancel();
        }
        self.pending = None;
    }

    /// Forgets the error of a generation (Reload).
    pub fn clear(&mut self) {
        self.cache = None;
        self.error = None;
        self.pending = None;
    }

    // ---- scrolling ---------------------------------------------------------------------------------------------

    /// The largest top row (the last row at the screen's bottom).
    pub fn max_top(&self, row_count: u64) -> u64 {
        row_count.saturating_sub(u64::from(self.rows_visible.max(1)))
    }

    /// Keeps `top` within the rows.
    pub fn clamp(&mut self, row_count: u64) {
        let max = self.max_top(row_count);
        if self.top > max || (self.top == max && self.frac > 0.0) {
            self.top = max;
            self.frac = 0.0;
        }
    }

    /// Scrolls by `rows` (fractional: the wheel's smooth part), within the rows.
    pub fn scroll_by(&mut self, rows: f64, row_count: u64) {
        let at = self.top as f64 + f64::from(self.frac) + rows;
        let max = self.max_top(row_count) as f64;
        let at = at.clamp(0.0, max);
        self.top = at.floor() as u64;
        self.frac = (at - at.floor()) as f32;
        self.clamp(row_count);
    }

    /// Pages by `n` screens.
    pub fn page(&mut self, n: i64, row_count: u64) {
        let page = i128::from(self.rows_visible.max(1));
        let at = (i128::from(self.top) + page * i128::from(n))
            .clamp(0, i128::from(self.max_top(row_count)));
        self.top = at as u64;
        self.frac = 0.0;
    }

    /// The top row of a thumb at `fraction` (0…1) of the trough, in `f64` (exact for 2⁴⁰ rows).
    pub fn thumb_to(&mut self, fraction: f64, row_count: u64) {
        let max = self.max_top(row_count);
        self.top = ((max as f64) * fraction.clamp(0.0, 1.0)).round() as u64;
        self.top = self.top.min(max);
        self.frac = 0.0;
    }

    /// Sets the top row (clamped).
    pub fn set_top(&mut self, top: u64, row_count: u64) {
        self.top = top.min(self.max_top(row_count));
        self.frac = 0.0;
    }

    /// Brings row `k` into view: when it is outside the screen, it becomes the row at the screen's upper third
    /// (`third`) or the nearest edge.
    pub fn reveal(&mut self, k: u64, row_count: u64, third: bool) {
        let visible = u64::from(self.rows_visible.max(1));
        let first_full = if self.frac > 0.0 {
            self.top + 1
        } else {
            self.top
        };
        let inside = k >= first_full && k < self.top + visible;
        if inside && !third {
            return;
        }
        let top = if third {
            k.saturating_sub(visible / 3)
        } else if k < first_full {
            k
        } else {
            k.saturating_sub(visible.saturating_sub(1))
        };
        self.set_top(top, row_count);
    }
}
