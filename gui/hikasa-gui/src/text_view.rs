//! The Text Compare view model (docs/05 §4–§6; contract §1.10, §3.7): it runs the comparison as a job, keeps
//! the display filter, the cursor and the scroll position, and computes the *visible slice* the panes draw
//! (ARCH-GUI-002: only the rows on screen are laid out; nothing exists per line of the documents).
//!
//! First screen (ARCH-GUI-003): `CompareObserver::loaded` hands over both documents before the alignment
//! starts, so the panes show the text unaligned at once; the final `Arc<TextComparison>` then replaces them
//! with aligned, coloured rows. Every decision about rows, sections, importance, the filter and the thumbnail
//! is a `TextComparison` call; this module only maps them to drawing data.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hikasa_core::{CancelToken, JobKind, Phase, Side};
use hikasa_session::api::{TextCompareRequest, TextCompareResponse, ViewOutcome};
use hikasa_session::{ApiError, Service};
use hikasa_text::{
    CharClass, CompareObserver, Direction, ElementId, FilteredView, Importance, PartialRows,
    RowIdx, RowState, TextDisplayFilter, TextDocument, TextToggles, ViewItem, ViewOptions,
};
use parking_lot::Mutex;
use serde::Serialize;

use crate::commands::{CommandId, Ctx};
use crate::job::Job;
use crate::status::{JobStatus, LogNote, StatusItems};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;

/// Default font size of the panes (px).
pub const DEFAULT_FONT_PX: u32 = 13;
/// Smallest and largest font sizes.
const FONT_RANGE: (u32, u32) = (8, 32);
/// Longest displayed line (UTF-16 units); longer lines are cut with a marker.
pub const MAX_DISPLAY_COLUMNS: usize = 4096;
/// Largest slice one request may ask for.
pub const MAX_SLICE: u32 = 512;
/// Syntax colour slots (the palette has this many element colours after the default colour 0).
pub const PALETTE_SLOTS: u32 = 8;

/// What the observer of the running comparison hands to the UI thread.
#[derive(Default)]
pub struct TextProbe {
    docs: Mutex<Option<(Arc<TextDocument>, Arc<TextDocument>)>>,
    phase: Mutex<Option<Phase>>,
    aligned: AtomicU64,
}

/// The `CompareObserver` of a view's job (runs on the job thread; returns at once).
struct Observer {
    probe: Arc<TextProbe>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl CompareObserver for Observer {
    fn loaded(&mut self, left: &Arc<TextDocument>, right: &Arc<TextDocument>) {
        *self.probe.docs.lock() = Some((Arc::clone(left), Arc::clone(right)));
        (self.wake)();
    }

    fn phase(&mut self, phase: Phase) {
        *self.probe.phase.lock() = Some(phase);
        (self.wake)();
    }

    fn partial(&mut self, rows: &PartialRows<'_>) {
        self.probe.aligned.store(
            u64::from(rows.left_done) + u64::from(rows.right_done),
            Ordering::Relaxed,
        );
    }
}

/// Phase of the view, as the panes see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "u8")]
pub enum TextPhase {
    /// Nothing to show yet.
    Loading,
    /// Both documents are loaded; the rows are not aligned yet.
    Loaded,
    /// The comparison is ready.
    Ready,
    /// The comparison failed or was cancelled.
    Failed,
}

impl From<TextPhase> for u8 {
    fn from(p: TextPhase) -> u8 {
        match p {
            TextPhase::Loading => 0,
            TextPhase::Loaded => 1,
            TextPhase::Ready => 2,
            TextPhase::Failed => 3,
        }
    }
}

/// Everything the panes need besides the rows (fetched when the view is first drawn and after every change).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextMeta {
    /// Data generation: cached slices of another generation are stale.
    #[serde(rename = "gen")]
    pub generation: u64,
    /// Display positions (rows and gap bars of the filtered view; unaligned: the longer document).
    pub len: u32,
    /// Alignment rows (thumbnail scale).
    pub rows: u32,
    /// Phase.
    pub phase: TextPhase,
    /// First visible position to restore.
    pub top: u32,
    /// Position of the cursor row (−1: none).
    pub cursor: i64,
    /// One-shot scroll request: a position to bring into view (−1: none).
    pub scroll: i64,
    /// Font size (px).
    pub font: u32,
    /// Thumbnail shown.
    pub thumb: bool,
    /// Syntax colours are in the slices (the spans are prepared).
    pub syntax: bool,
    /// Digits of the largest line number.
    pub digits: u32,
    /// Text for an empty pane (loading, errors, "no lines shown").
    pub message: String,
}

/// One display position of a slice. Codes: `k` 0 row, 1 gap bar, 2 unaligned row; `st` 0 same, 1 changed,
/// 2 left only, 3 right only; `im` 0 none, 1 unimportant, 2 important.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SliceRow {
    /// Kind.
    pub k: u8,
    /// Row state.
    pub st: u8,
    /// Importance.
    pub im: u8,
    /// Row index (gap: first hidden row).
    pub r: u32,
    /// Gap: hidden rows.
    pub g: u32,
    /// 1-based line numbers per side (0: no line).
    pub ln: [u32; 2],
    /// Displayed text per side (tabs expanded, controls made visible, cut at [`MAX_DISPLAY_COLUMNS`]).
    pub t: [Option<String>; 2],
    /// Syntax segments per side: `start, end, colour` triples in UTF-16 units covering the text.
    pub sp: [Vec<u32>; 2],
    /// Inline differences per side: `start, end, important` triples.
    pub hl: [Vec<u32>; 2],
}

/// A slice of display positions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSlice {
    /// Generation of the data.
    #[serde(rename = "gen")]
    pub generation: u64,
    /// First position.
    pub start: u32,
    /// The positions.
    pub rows: Vec<SliceRow>,
}

/// The thumbnail cells (TC-032): one byte of `ThumbCell` bits per bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThumbData {
    /// Generation.
    #[serde(rename = "gen")]
    pub generation: u64,
    /// Cells.
    pub cells: Vec<u8>,
}

/// File information of one pane (TC-035 header).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneInfo {
    /// Title or path.
    pub title: String,
    /// Encoding, line endings and line count (empty until loaded).
    pub detail: String,
}

/// What the status bar and the pane headers show.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextStatus {
    /// Left and right pane headers.
    pub panes: [PaneInfo; 2],
    /// Comparison state (left part of the status bar).
    pub state: String,
    /// Format, algorithm and timings (right part).
    pub detail: String,
    /// A load error.
    pub error: Option<String>,
}

/// The Text Compare view model.
pub struct TextViewModel {
    request: TextCompareRequest,
    titles: [Option<String>; 2],
    names: [String; 2],
    probe: Arc<TextProbe>,
    wake: Arc<dyn Fn() + Send + Sync>,
    job: Option<Job<TextCompareResponse>>,
    docs: Option<(Arc<TextDocument>, Arc<TextDocument>)>,
    response: Option<TextCompareResponse>,
    error: Option<String>,
    cancelled: bool,
    spans: SpanState,
    opts: ViewOptions,
    filtered: Option<FilteredView>,
    generation: u64,
    dirty: bool,
    cursor: Option<RowIdx>,
    top: u32,
    page: u32,
    scroll: Option<u32>,
    show_thumbnail: bool,
    font_px: u32,
    /// The theme's editor size (`DisplayFontReset`, D-113).
    default_font_px: u32,
    context_lines: u32,
    /// What the log learns when a comparison ends (D-116), taken by the app.
    notes: Vec<LogNote>,
    started: Instant,
    loaded_after: Option<Duration>,
    ready_after: Option<Duration>,
}

/// Preparation of the syntax spans (`TextComparison::prepare_spans` on a worker before the first `spans` call
/// of a multi-line grammar; §1.10).
enum SpanState {
    /// No comparison yet.
    Idle,
    /// The worker runs.
    Preparing(Job<()>),
    /// `spans` is O(line) from now on.
    Ready,
}

impl TextViewModel {
    /// Creates the view and starts its comparison job. `wake` posts a wake-up to the UI thread.
    pub fn new(
        service: &Service,
        request: TextCompareRequest,
        view: ViewOptions,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> TextViewModel {
        let names = [display_name(&request.left), display_name(&request.right)];
        let mut model = TextViewModel {
            request,
            titles,
            names,
            probe: Arc::new(TextProbe::default()),
            wake,
            job: None,
            docs: None,
            response: None,
            error: None,
            cancelled: false,
            spans: SpanState::Idle,
            opts: view,
            filtered: None,
            generation: 1,
            dirty: true,
            cursor: None,
            top: 0,
            page: 40,
            scroll: None,
            show_thumbnail: true,
            font_px: DEFAULT_FONT_PX,
            default_font_px: DEFAULT_FONT_PX,
            context_lines: service.options().context_lines,
            notes: Vec::new(),
            started: Instant::now(),
            loaded_after: None,
            ready_after: None,
        };
        model.start(service);
        model
    }

    /// (Re)starts the comparison (Reload).
    fn start(&mut self, service: &Service) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
        if let SpanState::Preparing(job) = std::mem::replace(&mut self.spans, SpanState::Idle) {
            job.cancel();
        }
        self.probe = Arc::new(TextProbe::default());
        self.docs = None;
        self.response = None;
        self.error = None;
        self.cancelled = false;
        self.filtered = None;
        self.cursor = None;
        self.started = Instant::now();
        self.loaded_after = None;
        self.ready_after = None;
        self.bump();
        let request = self.request.clone();
        let mut observer = Observer {
            probe: Arc::clone(&self.probe),
            wake: Arc::clone(&self.wake),
        };
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        self.job = Some(Job::spawn(
            service,
            JobKind::TextCompare,
            title,
            &self.wake,
            move |svc, job| svc.text_compare(&request, job, Some(&mut observer)),
        ));
    }

    /// The comparison request.
    #[cfg(test)]
    pub fn request(&self) -> &TextCompareRequest {
        &self.request
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&self.names[0], &self.names[1]],
        )
    }

    /// The probe of the running comparison (tests: it lives as long as the job's observer).
    #[cfg(test)]
    pub fn probe(&self) -> Arc<TextProbe> {
        Arc::clone(&self.probe)
    }

    /// The current view options (filter, ignore-unimportant, expanded gaps).
    #[cfg(test)]
    pub fn view_options(&self) -> &ViewOptions {
        &self.opts
    }

    /// The finished comparison, if any.
    pub fn response(&self) -> Option<&TextCompareResponse> {
        self.response.as_ref()
    }

    /// Time from the view's start to the loaded documents and to the finished comparison.
    pub fn timings(&self) -> (Option<Duration>, Option<Duration>) {
        (self.loaded_after, self.ready_after)
    }

    /// True while a job of the view runs.
    pub fn busy(&self) -> bool {
        self.job.is_some() || matches!(self.spans, SpanState::Preparing(_))
    }

    /// True while a job of the view has ended but its result is not published yet (the window looks again at
    /// once).
    pub fn settling(&self) -> bool {
        self.job.as_ref().is_some_and(Job::settling)
            || matches!(&self.spans, SpanState::Preparing(job) if job.settling())
    }

    /// True when the syntax spans can be drawn.
    pub fn spans_ready(&self) -> bool {
        matches!(self.spans, SpanState::Ready)
    }

    /// Stops every job (the view is closing).
    pub fn cancel(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
        if let SpanState::Preparing(job) = &self.spans {
            job.cancel();
        }
    }

    /// True once since the last call when the panes must refetch their meta.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn bump(&mut self) {
        self.generation += 1;
        self.dirty = true;
    }

    /// Polls the jobs (once per frame); true when something changed.
    pub fn poll(&mut self, service: &Service) -> bool {
        let mut changed = false;
        let arrived = if self.docs.is_none() && self.response.is_none() {
            self.probe.docs.lock().clone()
        } else {
            None
        };
        if let Some(docs) = arrived {
            self.docs = Some(docs);
            self.loaded_after = Some(self.started.elapsed());
            self.bump();
            changed = true;
        }
        if let Some(job) = &self.job
            && let Some(result) = job.try_result()
        {
            self.job = None;
            changed = true;
            match result {
                Ok(response) => {
                    self.finish(service, response);
                    if let Some(note) = self.summary_note() {
                        self.notes.push(note);
                    }
                }
                Err(ApiError::Cancelled) => {
                    self.cancelled = true;
                    self.notes.push(LogNote {
                        kind: LogKind::Summary,
                        text: "Comparison cancelled.".to_owned(),
                    });
                    self.bump();
                }
                Err(e) => {
                    self.notes.push(LogNote {
                        kind: LogKind::Error,
                        text: e.to_string(),
                    });
                    self.error = Some(e.to_string());
                    self.bump();
                }
            }
        }
        if let SpanState::Preparing(job) = &self.spans
            && let Some(result) = job.try_result()
        {
            self.spans = match result {
                Ok(()) => SpanState::Ready,
                Err(_) => SpanState::Idle,
            };
            self.bump();
            changed = true;
        }
        changed
    }

    fn finish(&mut self, service: &Service, response: TextCompareResponse) {
        self.ready_after = Some(self.started.elapsed());
        let comparison = Arc::clone(&response.comparison);
        self.docs = None;
        self.response = Some(response);
        self.rebuild();
        // A grammar without multi-line items needs no start states: spans are O(line) at once.
        if !comparison.format().grammar().has_multiline() {
            self.spans = SpanState::Ready;
        } else {
            self.spans = SpanState::Preparing(Job::spawn(
                service,
                JobKind::TextCompare,
                "syntax spans".into(),
                &self.wake,
                move |_svc, job| {
                    let cancel: &CancelToken = &job.cancel;
                    comparison
                        .prepare_spans(Side::Left, cancel)
                        .and_then(|()| comparison.prepare_spans(Side::Right, cancel))
                        .map_err(|_| ApiError::Cancelled)
                },
            ));
        }
        self.bump();
    }

    /// Rebuilds the filtered view after a filter, ignore-unimportant or gap change.
    fn rebuild(&mut self) {
        self.filtered = self
            .response
            .as_ref()
            .map(|r| r.comparison.view(&self.opts));
        if let (Some(cursor), Some(view)) = (self.cursor, &self.filtered) {
            let pos = match view.position_of(cursor) {
                Ok(pos) | Err(pos) => pos,
            };
            self.scroll = Some(pos);
        } else {
            self.scroll = Some(0);
            self.top = 0;
        }
        self.bump();
    }

    /// The phase.
    pub fn phase(&self) -> TextPhase {
        if self.response.is_some() {
            TextPhase::Ready
        } else if self.error.is_some() || self.cancelled {
            TextPhase::Failed
        } else if self.docs.is_some() {
            TextPhase::Loaded
        } else {
            TextPhase::Loading
        }
    }

    /// Number of display positions.
    pub fn len(&self) -> u32 {
        match (&self.filtered, &self.docs) {
            (Some(view), _) => view.len(),
            (None, Some((l, r))) => l.line_count().max(r.line_count()),
            _ => 0,
        }
    }

    /// The panes' meta (consumes the one-shot scroll request).
    pub fn meta(&mut self) -> TextMeta {
        let rows = self
            .response
            .as_ref()
            .map_or(0, |r| r.comparison.row_count());
        let lines = match (&self.response, &self.docs) {
            (Some(r), _) => r
                .comparison
                .document(Side::Left)
                .line_count()
                .max(r.comparison.document(Side::Right).line_count()),
            (None, Some((l, r))) => l.line_count().max(r.line_count()),
            _ => 0,
        };
        let cursor = match (self.cursor, &self.filtered) {
            (Some(c), Some(view)) => view.position_of(c).map_or(-1, i64::from),
            _ => -1,
        };
        let message = match self.phase() {
            TextPhase::Loading => "Loading…".to_owned(),
            TextPhase::Failed => self
                .error
                .clone()
                .unwrap_or_else(|| "The comparison was cancelled.".to_owned()),
            _ if lines == 0 => "Both files are empty.".to_owned(),
            _ if self.len() == 0 => "No lines are shown by the display filter.".to_owned(),
            _ => String::new(),
        };
        TextMeta {
            generation: self.generation,
            len: self.len(),
            rows,
            phase: self.phase(),
            top: self.top,
            cursor,
            scroll: self.scroll.take().map_or(-1, i64::from),
            font: self.font_px,
            thumb: self.show_thumbnail,
            syntax: self.response.is_some() && self.spans_ready(),
            digits: digits(lines),
            message,
        }
    }

    /// The panes report their viewport (first visible position, visible positions).
    pub fn set_viewport(&mut self, top: u32, page: u32) {
        self.top = top.min(self.len());
        self.page = page.max(1);
    }

    /// A click on a position: a gap bar expands (TC-035), a row becomes the cursor. True when the data changed.
    pub fn click(&mut self, pos: u32) -> bool {
        let Some(view) = &self.filtered else {
            return false;
        };
        match view.get(pos) {
            Some(ViewItem::Gap { rows }) => {
                self.opts.expanded.push(rows.clone());
                self.cursor = Some(RowIdx(rows.start));
                self.rebuild();
                true
            }
            Some(ViewItem::Row { row }) => {
                self.cursor = Some(row);
                false
            }
            None => false,
        }
    }

    /// The cursor row.
    #[cfg(test)]
    pub fn cursor(&self) -> Option<RowIdx> {
        self.cursor
    }

    /// Display positions `start..start + count` (clamped to [`MAX_SLICE`]).
    pub fn slice(&self, start: u32, count: u32) -> TextSlice {
        let end = start.saturating_add(count.min(MAX_SLICE)).min(self.len());
        let mut rows = Vec::with_capacity(end.saturating_sub(start) as usize);
        match (&self.response, &self.filtered, &self.docs) {
            (Some(response), Some(view), _) => {
                let cmp = &response.comparison;
                let tab = usize::from(cmp.format().text().tab_size.max(1));
                let syntax = self.spans_ready();
                for pos in start..end {
                    match view.get(pos) {
                        Some(ViewItem::Row { row }) => {
                            rows.push(self.aligned_row(cmp, row, tab, syntax));
                        }
                        Some(ViewItem::Gap { rows: hidden }) => rows.push(SliceRow {
                            k: 1,
                            st: 0,
                            im: 0,
                            r: hidden.start,
                            g: hidden.end - hidden.start,
                            ln: [0, 0],
                            t: [None, None],
                            sp: [Vec::new(), Vec::new()],
                            hl: [Vec::new(), Vec::new()],
                        }),
                        None => break,
                    }
                }
            }
            (None, None, Some((left, right))) => {
                for pos in start..end {
                    let side = |doc: &TextDocument| {
                        (pos < doc.line_count()).then(|| {
                            let layout =
                                layout_line(&doc.line(hikasa_text::document::LineIdx(pos)), 4);
                            let n = layout.units();
                            (
                                layout.text,
                                vec![0, n, 0],
                                doc.physical_line(hikasa_text::document::LineIdx(pos)),
                            )
                        })
                    };
                    let (l, r) = (side(left), side(right));
                    let ln = [l.as_ref().map_or(0, |x| x.2), r.as_ref().map_or(0, |x| x.2)];
                    let (lt, ls) = l.map_or((None, Vec::new()), |x| (Some(x.0), x.1));
                    let (rt, rs) = r.map_or((None, Vec::new()), |x| (Some(x.0), x.1));
                    rows.push(SliceRow {
                        k: 2,
                        st: 0,
                        im: 0,
                        r: pos,
                        g: 0,
                        ln,
                        t: [lt, rt],
                        sp: [ls, rs],
                        hl: [Vec::new(), Vec::new()],
                    });
                }
            }
            _ => {}
        }
        TextSlice {
            generation: self.generation,
            start,
            rows,
        }
    }

    fn aligned_row(
        &self,
        cmp: &hikasa_text::TextComparison,
        row: RowIdx,
        tab: usize,
        syntax: bool,
    ) -> SliceRow {
        let Some(r) = cmp.row(row) else {
            return SliceRow {
                k: 0,
                st: 0,
                im: 0,
                r: row.0,
                g: 0,
                ln: [0, 0],
                t: [None, None],
                sp: [Vec::new(), Vec::new()],
                hl: [Vec::new(), Vec::new()],
            };
        };
        let inline = match r.state {
            RowState::Changed => cmp.inline(row),
            _ => None,
        };
        let mut out = SliceRow {
            k: 0,
            st: match r.state {
                RowState::Same => 0,
                RowState::Changed => 1,
                RowState::LeftOnly => 2,
                RowState::RightOnly => 3,
            },
            im: match r.importance {
                Importance::None => 0,
                Importance::Unimportant => 1,
                Importance::Important => 2,
            },
            r: row.0,
            g: 0,
            ln: [0, 0],
            t: [None, None],
            sp: [Vec::new(), Vec::new()],
            hl: [Vec::new(), Vec::new()],
        };
        for (k, (side, line)) in [(Side::Left, r.left), (Side::Right, r.right)]
            .into_iter()
            .enumerate()
        {
            let Some(line) = line else { continue };
            let doc = cmp.document(side);
            let text = doc.line(line);
            let layout = layout_line(&text, tab);
            out.ln[k] = doc.physical_line(line);
            out.sp[k] = if syntax {
                let spans = cmp.spans(side, line);
                segments(
                    &layout,
                    spans.iter().map(|s| {
                        let colour = match s.class {
                            CharClass::Element if s.element != ElementId::EVERYTHING_ELSE => {
                                colour_slot(s.element)
                            }
                            _ => 0,
                        };
                        (s.range.start, s.range.end, colour)
                    }),
                )
            } else {
                vec![0, layout.units(), 0]
            };
            if let Some(inline) = &inline {
                let ranges = if k == 0 { &inline.left } else { &inline.right };
                let mut hl = Vec::with_capacity(ranges.len() * 3);
                for range in ranges {
                    let (s, e) = (layout.col(range.range.start), layout.col(range.range.end));
                    if e > s || range.range.start == range.range.end {
                        hl.extend([s, e.max(s), u32::from(range.important)]);
                    }
                }
                out.hl[k] = hl;
            }
            out.t[k] = Some(layout.text);
        }
        out
    }

    /// The thumbnail with `height` buckets (TC-032).
    pub fn thumbnail(&self, height: u32) -> ThumbData {
        let cells = self.response.as_ref().map_or_else(Vec::new, |r| {
            r.comparison
                .thumbnail(height.clamp(1, 8192), self.opts.ignore_unimportant)
                .into_iter()
                .map(|c| c.bits())
                .collect()
        });
        ThumbData {
            generation: self.generation,
            cells,
        }
    }

    /// The display position of a row (thumbnail clicks).
    pub fn position_of_row(&self, row: u32) -> u32 {
        match &self.filtered {
            Some(view) => match view.position_of(RowIdx(row)) {
                Ok(p) | Err(p) => p,
            },
            None => row,
        }
    }

    /// The row at the top of the viewport.
    fn top_row(&self) -> Option<u32> {
        let view = self.filtered.as_ref()?;
        match view.get(self.top)? {
            ViewItem::Row { row } => Some(row.0),
            ViewItem::Gap { rows } => Some(rows.start),
        }
    }

    /// The section a "next/previous difference section" command goes to (TC-050), as its first row. From the
    /// cursor the core's `next_section`; without a cursor `next_section_from` the top row (a section starting
    /// on the top row counts as "next").
    fn section_target(&self, dir: Direction) -> Option<u32> {
        let cmp = &self.response.as_ref()?.comparison;
        let iu = self.opts.ignore_unimportant;
        let id = match self.cursor {
            Some(cursor) => cmp.next_section(cursor, dir, iu)?,
            None => cmp.next_section_from(RowIdx(self.top_row()?), dir, iu)?,
        };
        cmp.sections().get(id.0 as usize).map(|s| s.rows.start)
    }

    fn go_to_section(&mut self, dir: Direction) -> bool {
        let Some(row) = self.section_target(dir) else {
            return false;
        };
        self.cursor = Some(RowIdx(row));
        self.scroll = Some(self.position_of_row(row));
        self.dirty = true;
        true
    }

    /// Fills the command context of this view.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.busy = self.busy();
        ctx.ready = self.response.is_some();
        ctx.next_difference = self.section_target(Direction::Forward).is_some();
        ctx.prev_difference = self.section_target(Direction::Backward).is_some();
    }

    /// The atomic toggles the current filter shows (TC-040; the core's table).
    fn toggles(&self) -> TextToggles {
        self.opts.filter.toggles()
    }

    fn set_filter(&mut self, filter: TextDisplayFilter) {
        self.opts.filter = filter;
        self.opts.expanded.clear();
        self.rebuild();
    }

    fn toggle(&mut self, bit: TextToggles) {
        self.set_filter(self.opts.filter.toggled(bit));
    }

    /// The check state of a toggle or radio command (`None`: not a checkable command of this view).
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        let filter = self.opts.filter;
        Some(match id {
            CommandId::FavCompareAll => filter == TextDisplayFilter::All,
            CommandId::FavDiff => filter == TextDisplayFilter::Differences,
            CommandId::FavSame => filter == TextDisplayFilter::Same,
            CommandId::FavCompareNone => filter == TextDisplayFilter::None,
            CommandId::ShowContext => matches!(filter, TextDisplayFilter::Context { .. }),
            CommandId::TogSame => self.toggles().contains(TextToggles::SAME),
            CommandId::TogLeftOrphan => self.toggles().contains(TextToggles::LEFT_ORPHANS),
            CommandId::TogRightOrphan => self.toggles().contains(TextToggles::RIGHT_ORPHANS),
            CommandId::TogDiff => self.toggles().contains(TextToggles::DIFFERENCES),
            CommandId::InsigIgnored => self.opts.ignore_unimportant,
            CommandId::ShowThumbnail => self.show_thumbnail,
            _ => return None,
        })
    }

    /// Executes a command of this view; false when the id is not one of its commands.
    pub fn execute(&mut self, service: &Service, id: CommandId) -> bool {
        match id {
            CommandId::Reload => self.start(service),
            CommandId::DiffSectionNext => {
                self.go_to_section(Direction::Forward);
            }
            CommandId::DiffSectionPrev => {
                self.go_to_section(Direction::Backward);
            }
            CommandId::FavCompareAll => self.set_filter(TextDisplayFilter::All),
            CommandId::FavDiff => self.set_filter(TextDisplayFilter::Differences),
            CommandId::FavSame => self.set_filter(TextDisplayFilter::Same),
            CommandId::FavCompareNone => self.set_filter(TextDisplayFilter::None),
            CommandId::ShowContext => self.set_filter(TextDisplayFilter::Context {
                lines: self.context_lines,
            }),
            CommandId::TogSame => self.toggle(TextToggles::SAME),
            CommandId::TogLeftOrphan => self.toggle(TextToggles::LEFT_ORPHANS),
            CommandId::TogRightOrphan => self.toggle(TextToggles::RIGHT_ORPHANS),
            CommandId::TogDiff => self.toggle(TextToggles::DIFFERENCES),
            CommandId::InsigIgnored => {
                self.opts.ignore_unimportant = !self.opts.ignore_unimportant;
                self.opts.expanded.clear();
                self.rebuild();
            }
            CommandId::ShowThumbnail => {
                self.show_thumbnail = !self.show_thumbnail;
                self.dirty = true;
            }
            CommandId::DisplayFontIncrease => self.set_font(self.font_px + 1),
            CommandId::DisplayFontDecrease => self.set_font(self.font_px.saturating_sub(1)),
            CommandId::DisplayFontReset => self.set_font(self.default_font_px),
            _ => return false,
        }
        true
    }

    /// The current font size of the panes (px).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn font_px(&self) -> u32 {
        self.font_px
    }

    /// The thumbnail is shown.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn show_thumbnail(&self) -> bool {
        self.show_thumbnail
    }

    /// Applies the layout of the view type (D-108): `font_size` (else the theme's editor size `default_px`) and
    /// `thumbnail`. A value of the wrong type is ignored.
    pub fn apply_layout(&mut self, layout: &State, default_px: u32) {
        let view = crate::commands::Scope::Text.view_name().unwrap_or_default();
        self.default_font_px = default_px.clamp(FONT_RANGE.0, FONT_RANGE.1);
        let size = layout
            .layout_value(view, "font_size")
            .and_then(LayoutValue::as_float)
            .map_or(self.default_font_px, |f| {
                f.round().clamp(1.0, 1000.0) as u32
            });
        self.set_font(size);
        if let Some(on) = layout
            .layout_value(view, "thumbnail")
            .and_then(LayoutValue::as_bool)
        {
            self.show_thumbnail = on;
        }
    }

    /// The layout keys this view owns (D-108): `font_size`, `thumbnail`.
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("font_size", LayoutValue::Float(f64::from(self.font_px))),
            ("thumbnail", LayoutValue::Bool(self.show_thumbnail)),
        ]
    }

    /// The inputs watched for changes on disk (D-118): the two specs as given.
    pub fn inputs(&self) -> [String; 2] {
        [lossy(&self.request.left), lossy(&self.request.right)]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, D-107).
    pub fn duplicate(&self) -> (hikasa_session::api::ViewRequest, [Option<String>; 2]) {
        (
            hikasa_session::api::ViewRequest::Text {
                request: self.request.clone(),
                view: self.opts.clone(),
            },
            self.titles.clone(),
        )
    }

    /// The running jobs (the job indicator, D-115).
    pub fn jobs(&self, view: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
        let mut out = Vec::new();
        if let Some(job) = &self.job {
            let (phase, fraction) = job.progress();
            out.push(JobStatus {
                view,
                title: job.title().to_owned(),
                phase: phase.or_else(|| *self.probe.phase.lock()),
                fraction,
            });
        }
        if let SpanState::Preparing(job) = &self.spans {
            out.push(JobStatus {
                view,
                title: job.title().to_owned(),
                phase: None,
                fraction: None,
            });
        }
        out
    }

    /// The notes for the log since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    /// The summary of a finished comparison for the log: the verdict, the sections and the duration.
    fn summary_note(&self) -> Option<LogNote> {
        let r = self.response.as_ref()?;
        let stats = r.comparison.stats();
        let verdict = r.comparison.verdict(self.opts.ignore_unimportant).kind();
        Some(LogNote {
            kind: LogKind::Summary,
            text: format!(
                "{}: {} important and {} unimportant of {} sections, compared in {} ms.",
                verdict_text(verdict),
                stats.important_sections,
                stats.sections - stats.important_sections,
                stats.sections,
                self.ready_after.map_or(0, |d| d.as_millis()),
            ),
        })
    }

    /// The status bar (D-115): the comparison state on the left; the format, then each side's encoding and line
    /// endings on the right.
    pub fn status_items(&self) -> StatusItems {
        let status = self.status();
        let mut right = Vec::new();
        if let Some(r) = &self.response {
            right.push(r.format.name.clone());
        }
        for pane in &status.panes {
            if !pane.detail.is_empty() {
                right.push(pane.detail.clone());
            }
        }
        StatusItems {
            left: status.state,
            right,
        }
    }

    fn set_font(&mut self, px: u32) {
        self.font_px = px.clamp(FONT_RANGE.0, FONT_RANGE.1);
        self.dirty = true;
    }

    /// The D-23 outcome when the view closes (§1.8.7 item 22): the verdict under the view's current
    /// ignore-unimportant, `LoadFailed` when the inputs could not be compared, `NoVerdict` while comparing or
    /// after a cancellation.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        // A result that arrived since the last frame counts.
        self.poll(service);
        if let Some(response) = &self.response {
            let iu = self.opts.ignore_unimportant;
            ViewOutcome::Text {
                result: response.comparison.verdict(iu),
                ignore_unimportant: iu,
            }
        } else if self.error.is_some() {
            ViewOutcome::LoadFailed
        } else {
            ViewOutcome::NoVerdict
        }
    }

    /// Pane headers and status bar texts.
    pub fn status(&self) -> TextStatus {
        let mut status = TextStatus {
            panes: [(0, &self.request.left), (1, &self.request.right)].map(|(k, spec)| PaneInfo {
                title: crate::shared::shown_path(
                    &self.titles[k].clone().unwrap_or_else(|| lossy(spec)),
                ),
                detail: String::new(),
            }),
            error: self.error.as_deref().map(crate::shared::shown_path),
            ..TextStatus::default()
        };
        let docs = match (&self.response, &self.docs) {
            (Some(r), _) => Some((
                Arc::clone(r.comparison.document(Side::Left)),
                Arc::clone(r.comparison.document(Side::Right)),
            )),
            (None, Some((l, r))) => Some((Arc::clone(l), Arc::clone(r))),
            _ => None,
        };
        if let Some((l, r)) = docs {
            status.panes[0].detail = doc_detail(&l);
            status.panes[1].detail = doc_detail(&r);
        }
        status.state = match (&self.response, self.phase()) {
            (Some(r), _) => {
                let stats = r.comparison.stats();
                let iu = self.opts.ignore_unimportant;
                let verdict = r.comparison.verdict(iu).kind();
                format!(
                    "{} · {} important and {} unimportant of {} sections · {} rows",
                    verdict_text(verdict),
                    stats.important_sections,
                    stats.sections - stats.important_sections,
                    stats.sections,
                    stats.rows
                )
            }
            (None, TextPhase::Loading) => "Loading the files…".to_owned(),
            (None, TextPhase::Loaded) => {
                let phase = self.probe.phase.lock().map_or("comparing", Phase::as_str);
                format!("Comparing ({})…", phase.replace('_', " "))
            }
            (None, TextPhase::Failed) if self.cancelled => "Cancelled.".to_owned(),
            (None, _) => "The files could not be compared.".to_owned(),
        };
        if let Some(r) = &self.response {
            let syntax = if self.spans_ready() {
                ""
            } else {
                " · preparing syntax colours"
            };
            status.detail = format!(
                "{} · {:?} · loaded in {} ms, compared in {} ms{syntax}",
                r.format.name,
                r.comparison.algorithm(),
                self.loaded_after.map_or(0, |d| d.as_millis()),
                self.ready_after.map_or(0, |d| d.as_millis()),
            );
        }
        status
    }
}

/// Encoding, line endings and line count of a document.
fn doc_detail(doc: &TextDocument) -> String {
    use hikasa_text::document::EolStyle;
    let eol = match doc.eol().style() {
        EolStyle::Lf => "LF",
        EolStyle::CrLf => "CRLF",
        EolStyle::Cr => "CR",
        EolStyle::Mixed => "mixed line endings",
        EolStyle::None => "no line ending",
    };
    format!(
        "{} · {} · {} lines",
        doc.encoding().encoding.name(),
        eol,
        doc.line_count()
    )
}

/// Words of the verdict for the status bar.
fn verdict_text(kind: hikasa_core::ContentKind) -> &'static str {
    use hikasa_core::ContentKind as K;
    match kind {
        K::BinarySame => "Identical",
        K::RulesSame => "No important differences",
        K::UnimportantOnly => "Only unimportant differences",
        K::ImportantDifferences => "Important differences",
        K::BinaryDifferent => "Binary differences",
        K::Error => "Error",
    }
}

/// Digits of the largest line number (gutter width).
fn digits(lines: u32) -> u32 {
    lines.max(1).ilog10() + 1
}

/// The colour slot of a grammar element: elements are numbered in order of first use in the grammar (D-41),
/// so each element of a format keeps its colour.
pub fn colour_slot(element: ElementId) -> u32 {
    (u32::from(element.0).saturating_sub(1) % PALETTE_SLOTS) + 1
}

/// The last path component of a spec, for captions.
fn display_name(spec: &std::ffi::OsStr) -> String {
    crate::shared::spec_name(&spec.to_string_lossy()).to_owned()
}

fn lossy(spec: &std::ffi::OsStr) -> String {
    spec.to_string_lossy().into_owned()
}

/// A line laid out for the panes: tabs expanded, C0 controls and DEL shown as control pictures, other invisible
/// characters ([`is_invisible`]) as `\u{…}` escapes, cut at
/// [`MAX_DISPLAY_COLUMNS`]; `map` gives the displayed UTF-16 column of every char boundary of the source.
pub struct Layout {
    /// Displayed text.
    pub text: String,
    map: Vec<(u32, u32)>,
    units: u32,
}

impl Layout {
    /// Displayed length in UTF-16 units.
    pub fn units(&self) -> u32 {
        self.units
    }

    /// Displayed column of source byte offset `byte` (clamped to the displayed text).
    pub fn col(&self, byte: u32) -> u32 {
        let k = self.map.partition_point(|&(b, _)| b <= byte);
        match k.checked_sub(1).and_then(|i| self.map.get(i)) {
            Some(&(_, col)) => col.min(self.units),
            None => 0,
        }
    }
}

/// True for the characters the panes show as `\u{…}` escapes because they draw nothing, or reorder or break the
/// text around them: the C1 controls and the bidirectional formatting characters that the reports escape
/// (`hikasa_report::escape_controls`), and besides the zero-width characters (U+200B–U+200D, U+2060), the line
/// and paragraph separators (U+2028, U+2029) and the byte order mark (U+FEFF). C0 controls and DEL are drawn as
/// control pictures (`␛`) by [`layout_line`].
pub fn is_invisible(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut buf = [0u8; 4];
    matches!(
        hikasa_report::escape_controls(c.encode_utf8(&mut buf)),
        std::borrow::Cow::Owned(_)
    ) || matches!(
        c,
        '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{2028}' | '\u{2029}' | '\u{feff}'
    )
}

/// Lays out `text` for display with tab stops every `tab` columns.
pub fn layout_line(text: &str, tab: usize) -> Layout {
    let tab = tab.max(1);
    let mut out = String::with_capacity(text.len().min(MAX_DISPLAY_COLUMNS) + 8);
    let mut map = Vec::with_capacity(text.len().min(MAX_DISPLAY_COLUMNS) + 1);
    let mut units = 0usize;
    let mut cut = false;
    for (byte, c) in text.char_indices() {
        if units >= MAX_DISPLAY_COLUMNS {
            cut = true;
            map.push((byte as u32, units as u32));
            break;
        }
        map.push((byte as u32, units as u32));
        match c {
            '\t' => {
                let n = tab - units % tab;
                out.extend(std::iter::repeat_n(' ', n));
                units += n;
            }
            '\u{0}'..='\u{1f}' => {
                out.push(char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'));
                units += 1;
            }
            '\u{7f}' => {
                out.push('\u{2421}');
                units += 1;
            }
            c if is_invisible(c) => {
                // `\u{…}` like the reports' escapes; every char of the escape is ASCII (one unit).
                let escape = format!("\\u{{{:x}}}", u32::from(c));
                units += escape.len();
                out.push_str(&escape);
            }
            _ => {
                out.push(c);
                units += c.len_utf16();
            }
        }
    }
    if !cut {
        map.push((text.len() as u32, units as u32));
    } else {
        out.push('…');
        units += 1;
    }
    Layout {
        text: out,
        map,
        units: units as u32,
    }
}

/// Coloured segments covering the displayed text: `(start, end, colour)` source byte ranges → merged
/// `start, end, colour` triples in display columns; uncovered text gets colour 0.
fn segments(layout: &Layout, spans: impl Iterator<Item = (u32, u32, u32)>) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut at = 0u32;
    let push = |out: &mut Vec<u32>, s: u32, e: u32, c: u32| {
        if e <= s {
            return;
        }
        let n = out.len();
        if n >= 3 && out[n - 1] == c && out[n - 2] == s {
            out[n - 2] = e;
        } else {
            out.extend([s, e, c]);
        }
    };
    for (start, end, colour) in spans {
        let (s, e) = (layout.col(start), layout.col(end));
        if s > at {
            push(&mut out, at, s, 0);
        }
        let s = s.max(at);
        push(&mut out, s, e, colour);
        at = at.max(e);
    }
    if at < layout.units() {
        push(&mut out, at, layout.units(), 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_expands_tabs_and_maps_offsets() {
        let l = layout_line("a\tb\u{1}é", 4);
        assert_eq!(l.text, "a   b\u{2401}é");
        assert_eq!(l.units(), 7);
        assert_eq!(l.col(0), 0);
        assert_eq!(l.col(1), 1);
        assert_eq!(l.col(2), 4);
        assert_eq!(l.col(3), 5);
        assert_eq!(l.col(4), 6);
        assert_eq!(l.col(6), 7);
        assert_eq!(l.col(99), 7);
    }

    #[test]
    fn invisible_characters_become_escapes() {
        // C1 (NEL, CSI), bidi (RLO, PDF, LRI, PDI, ALM), zero-width (ZWSP, ZWJ, WJ), LS, PS and BOM.
        for c in [
            '\u{85}', '\u{9b}', '\u{202e}', '\u{202c}', '\u{2066}', '\u{2069}', '\u{61c}',
            '\u{200b}', '\u{200d}', '\u{2060}', '\u{2028}', '\u{2029}', '\u{feff}',
        ] {
            assert!(is_invisible(c), "{c:?}");
        }
        for c in ['a', '\t', '\u{1b}', 'é', '\u{a0}', '中', '😀', '\u{2400}'] {
            assert!(!is_invisible(c), "{c:?}");
        }
        // "ZW:\u{200b}x\u{202e}y" with the escapes in step with the column map.
        let l = layout_line("ZW:\u{200b}x\u{202e}y\u{1b}", 4);
        assert_eq!(l.text, "ZW:\\u{200b}x\\u{202e}y\u{241b}");
        assert_eq!(l.units(), 3 + 8 + 1 + 8 + 1 + 1);
        assert_eq!(l.col(3), 3, "the escape starts where the character was");
        assert_eq!(l.col(6), 11, "the next character follows the escape");
        assert_eq!(l.col(7), 12);
        assert_eq!(l.col(10), 20);
        assert_eq!(l.col(12), 22);
    }

    #[test]
    fn long_lines_are_cut_with_a_marker() {
        let text = "x".repeat(MAX_DISPLAY_COLUMNS + 10);
        let l = layout_line(&text, 4);
        assert_eq!(l.units() as usize, MAX_DISPLAY_COLUMNS + 1);
        assert!(l.text.ends_with('…'));
        assert_eq!(l.col(text.len() as u32) as usize, MAX_DISPLAY_COLUMNS);
    }

    #[test]
    fn segments_cover_the_line_and_merge_colours() {
        let l = layout_line("int x = 1;", 4);
        let seg = segments(&l, [(0, 3, 2), (4, 5, 0), (8, 9, 3)].into_iter());
        assert_eq!(seg, vec![0, 3, 2, 3, 8, 0, 8, 9, 3, 9, 10, 0]);
        assert_eq!(segments(&l, std::iter::empty()), vec![0, 10, 0]);
    }

    #[test]
    fn names_and_digits() {
        assert_eq!(display_name(std::ffi::OsStr::new("/w/src/a.c")), "a.c");
        assert_eq!(display_name(std::ffi::OsStr::new("/w/dir/")), "dir");
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(1_000_000), 7);
        assert_eq!(colour_slot(ElementId(1)), 1);
        assert_eq!(colour_slot(ElementId(9)), 1);
    }
}
