//! The Hex Compare view model (M2 W13a, contract §1.13.3–§1.13.8, D-355…D-360), toolkit-free.
//!
//! The view opens both documents in a job (`Service::hex_open`, no pass over the bytes), shows None's provisional rows
//! at once (`HexView::provisional`), then aligns in a job with the session's mode (`Service::hex_align`; Fast by
//! default) and replaces the rows in one frame when the alignment lands, keeping the caret's byte and the top row's
//! byte. Every row, byte state, section, navigation target, address text, value, find hit and copied text comes from
//! core (`HexView`, `nav`, `values`, `copy`, `HexFinder`, the Service): this module places them and never decides one
//! (architecture check 7). Anything that reads a file runs in a job; the UI thread runs only calls bounded by the
//! screen (`try_rows`, `nav::step`, `values::at` of cached bytes).

pub mod copy;
pub mod find;
pub mod info;
pub mod layout;
pub mod nav;
pub mod pane;
pub mod screen;
pub mod status;
pub mod thumbnail;
pub mod values;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{ContentKind, ContentResult, JobKind, Side, Warning};
use hikasa_hex::nav::Step;
use hikasa_hex::{AlignMode, DisplayFilter, HexAlignOptions, HexAlignment, HexLayout, Radix};
use hikasa_session::api::{
    HexCompareRequest, HexCompareResponse, HexOpened, HexValues, ViewOutcome, ViewRequest,
};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;
use hikasa_session::{ApiError, JobControl, Service};

use crate::commands::{CommandId, Ctx};
use crate::job::{Job, Wake};
use crate::status::{JobStatus, LogNote, Message};

use self::pane::{CaretKey, Hit, PaneState, Part};
use self::screen::{Screen, Source};

/// The scope name of the view type in `state.toml` and in sessions' `[view]`.
pub const VIEW: &str = "hex-compare";
/// Default font size of the panes (px).
pub const DEFAULT_FONT_PX: u32 = 13;
/// The splitter's range (the text views', D-305).
pub const SPLIT_RANGE: (f32, f32) = (0.1, 0.9);
/// Smallest and largest font sizes.
const FONT_RANGE: (u32, u32) = (6, 48);
/// How long a closing view waits for its alignment (W11's rule, D-294).
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// The message of a filter before the alignment lands (D-355's open default).
pub const FILTER_LATER: &str = "The filter applies when the comparison is complete.";
/// The message of Next Difference without one.
pub const NO_MORE: &str = "No more differences.";

/// How the two panes are arranged (`[view] layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneLayout {
    /// Left beside right.
    #[default]
    SideBySide,
    /// Left above right.
    OverUnder,
}

impl PaneLayout {
    /// The `[view] layout` word.
    pub fn word(self) -> &'static str {
        match self {
            PaneLayout::SideBySide => "side-by-side",
            PaneLayout::OverUnder => "over-under",
        }
    }
}

/// The display options of a view: the layout of the rows and what the view shows (`[view]`, `[display]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexDisplay {
    /// Bytes per line, group, characters, address radix.
    pub layout: HexLayout,
    /// The address column is shown (`[view] addresses`).
    pub addresses: bool,
    /// The display filter (`[display] filter`).
    pub filter: DisplayFilter,
    /// Context rows of `differences` (0…64).
    pub context: u8,
    /// The panes' arrangement.
    pub panes: PaneLayout,
    /// The thumbnail strip (`[view] thumbnail`).
    pub thumbnail: bool,
    /// The value panel (`[view] values`).
    pub values: HexValues,
    /// The value panel's integers (`[view] values_radix`).
    pub values_radix: Radix,
    /// Font size (px).
    pub font_px: u32,
    /// The File Info headers (`ShowSpecInfo`).
    pub headers: bool,
    /// The splitter: the left (upper) pane's share of the panes' width (height), `[layout.hex-compare] splitter`.
    pub split: f32,
}

impl Default for HexDisplay {
    fn default() -> HexDisplay {
        HexDisplay {
            layout: HexLayout::default(),
            addresses: true,
            filter: DisplayFilter::All,
            context: 0,
            panes: PaneLayout::SideBySide,
            thumbnail: true,
            values: HexValues::None,
            values_radix: Radix::Decimal,
            font_px: DEFAULT_FONT_PX,
            headers: true,
            split: 0.5,
        }
    }
}

/// The phase of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexPhase {
    /// No specs yet: the headers take them.
    Empty,
    /// The documents are being opened.
    Opening,
    /// Provisional rows: the alignment runs or was stopped.
    Provisional,
    /// The alignment landed.
    Aligned,
    /// A side could not be opened.
    Failed,
}

/// What the view asks of its window (drained after each command and each poll).
#[derive(Debug, Clone, PartialEq)]
pub enum HexRequest {
    /// Writes this text to the clipboard (the window's ordered clipboard, W9's rule); the message after it.
    Clipboard(String, String),
    /// Writes the primary selection (X11, Wayland).
    Primary(String),
    /// A message panel line (`No more differences` beeps without the message panel, D-309).
    Message(Message),
    /// A notice: a warning with its log entry; `reload` offers *Reload* (D-118).
    Notice {
        /// The text.
        text: String,
        /// *Reload* is offered.
        reload: bool,
    },
}

/// A running search: its side, where it started, its direction and the job.
type FindJob = (Side, u64, hikasa_hex::Direction, Job<Option<Range<u64>>>);

/// What the alignment job brings back.
pub struct Landed {
    alignment: Arc<HexAlignment>,
    raw_same: Option<bool>,
    changed: Vec<Warning>,
}

/// What runs an alignment (`Service::hex_align`; tests hold it with a gate).
pub type Aligner = Arc<
    dyn Fn(
            &Service,
            &JobControl,
            &HexOpened,
            &HexAlignOptions,
        ) -> Result<Arc<HexAlignment>, ApiError>
        + Send
        + Sync,
>;

/// Calls the view made (tests count them: a rule change realigns without reopening).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HexCounters {
    /// Value panel jobs cancelled (a newer byte, a cached byte, the panel hidden, Clear Session).
    pub values_cancelled: u32,
    /// `hex_open` jobs started.
    pub opens: u32,
    /// `hex_align` jobs started.
    pub aligns: u32,
    /// Alignment jobs cancelled by a newer one or by Esc.
    pub cancelled: u32,
}

/// One kept column: the column and each side's byte there (its offset, or the gap's next byte, and whether the column
/// holds it).
#[derive(Debug, Clone, Copy)]
struct Place {
    col: u64,
    sides: [(u64, bool); 2],
}

/// What a relocation keeps: the caret, the anchor and the top row's first column. A filter or layout change keeps the
/// columns (the alignment did not change); a landing maps them by their bytes (W13a review F1, F2).
#[derive(Debug, Clone, Copy)]
struct Kept {
    caret: Place,
    anchor: Place,
    top: Option<Place>,
}

/// The Hex Compare view model.
pub struct HexViewModel {
    service: Service,
    wake: Wake,
    request: HexCompareRequest,
    titles: [Option<String>; 2],
    names: [String; 2],
    align_options: HexAlignOptions,
    opened: Option<Arc<HexOpened>>,
    error: Option<(Option<Side>, String)>,
    alignment: Option<Arc<HexAlignment>>,
    response: Option<HexCompareResponse>,
    stale: bool,
    stopped: bool,
    /// + 1 per landed alignment, layout, filter or reopen: what the screen caches of another generation is stale.
    pub generation: u64,
    /// The requested mode (the landed one is the alignment's).
    pub mode: AlignMode,
    display: HexDisplay,
    default_font_px: u32,
    source: Option<Source>,
    /// The scroll model and the row cache.
    pub screen: Screen,
    panes: [PaneState; 2],
    focus: Side,
    open_job: Option<Job<HexOpened>>,
    align_job: Option<Job<Landed>>,
    scan_job: Option<Job<Option<Range<u64>>>>,
    find_job: Option<FindJob>,
    copy_job: Option<Job<String>>,
    /// The primary selection's text read in a job (its bytes were not cached when the selection settled).
    primary_job: Option<Job<String>>,
    /// The value panel's job: the side and offset it reads.
    values_job: Option<((Side, u64), Job<hikasa_hex::Values>)>,
    /// A navigation step waiting for the alignment (W11b's queued-navigation rule).
    queued: Option<Step>,
    /// The Find panel.
    pub find: find::FindPanel,
    /// The value panel's lines and whether they are stale (a job runs).
    value_lines: Option<(String, bool)>,
    /// The current find hit (`side`, bytes).
    hit: Option<(Side, Range<u64>)>,
    requests: Vec<HexRequest>,
    notes: Vec<LogNote>,
    aligner: Aligner,
    /// Calls made (tests).
    pub counters: HexCounters,
    write: crate::folder_view::WritePolicy,
    started: Instant,
    ready_after: Option<Duration>,
    /// The frame time (seconds, the window's clock): the primary selection settles on it.
    pub now: f64,
    primary_at: Option<f64>,
    /// Primary selection writes asked (tests).
    pub primary_written: u64,
    /// A pointer drag selects from this pane.
    dragging: Option<Side>,
    /// The panel with the keyboard (the window sets it each frame): F1 shows its topic.
    pub panel: Option<&'static str>,
    /// The value panel has the keyboard (the window sets it each frame): Copy copies its lines.
    pub values_focus: bool,
    /// The enabled Hex formats' names (the headers' Format ▸).
    format_names: Vec<String>,
}

impl HexViewModel {
    /// Creates the view and starts opening its documents. `request.rules` are the words the view runs with (the
    /// window puts them on the defaults session, D-374); `layout` and `filter` the view's rows.
    pub fn new(
        service: &Service,
        request: HexCompareRequest,
        layout: HexLayout,
        filter: Option<DisplayFilter>,
        titles: [Option<String>; 2],
        wake: Wake,
    ) -> HexViewModel {
        let aligner: Aligner = Arc::new(|svc, job, opened, opts| svc.hex_align(opened, opts, job));
        HexViewModel::with_aligner(service, request, layout, filter, titles, wake, aligner)
    }

    /// [`HexViewModel::new`] with the alignment run by `aligner`.
    pub fn with_aligner(
        service: &Service,
        request: HexCompareRequest,
        layout: HexLayout,
        filter: Option<DisplayFilter>,
        titles: [Option<String>; 2],
        wake: Wake,
        aligner: Aligner,
    ) -> HexViewModel {
        let names = [display_name(&request.left), display_name(&request.right)];
        let settings = service.hex_settings(&request);
        let mut display = HexDisplay {
            layout,
            filter: filter.unwrap_or(DisplayFilter::All),
            ..HexDisplay::default()
        };
        let mut align_options = HexAlignOptions::default();
        let mut error = None;
        match &settings {
            Ok(s) => {
                align_options = s.align;
                display.context = s.context;
            }
            Err(e) => error = Some((None, e.to_string())),
        }
        let mut model = HexViewModel {
            service: service.clone(),
            wake,
            mode: align_options.mode,
            request,
            titles,
            names,
            align_options,
            opened: None,
            error,
            alignment: None,
            response: None,
            stale: false,
            stopped: false,
            generation: 1,
            display,
            default_font_px: DEFAULT_FONT_PX,
            source: None,
            screen: Screen::default(),
            panes: [PaneState::new(Side::Left), PaneState::new(Side::Right)],
            focus: Side::Left,
            open_job: None,
            align_job: None,
            scan_job: None,
            find_job: None,
            copy_job: None,
            primary_job: None,
            values_job: None,
            queued: None,
            find: find::FindPanel::default(),
            value_lines: None,
            hit: None,
            requests: Vec::new(),
            notes: Vec::new(),
            aligner,
            counters: HexCounters::default(),
            write: crate::folder_view::WritePolicy::default(),
            started: Instant::now(),
            ready_after: None,
            now: 0.0,
            primary_at: None,
            primary_written: 0,
            dragging: None,
            panel: None,
            values_focus: false,
            format_names: Vec::new(),
        };
        if model.error.is_none() {
            model.open();
        }
        model
    }

    // ---- opening and aligning ------------------------------------------------------------------------------------

    /// Opens both documents in a job (Reload, a spec, a format or `unlocked_max` changed): the rows of the old
    /// documents stay until the new ones open.
    fn open(&mut self) {
        if self.is_empty() {
            return;
        }
        for job in [self.align_job.take()].into_iter().flatten() {
            job.cancel();
        }
        if let Some(job) = self.open_job.take() {
            job.cancel();
        }
        self.cancel_searches();
        self.error = None;
        self.stopped = false;
        self.started = Instant::now();
        self.ready_after = None;
        self.counters.opens += 1;
        let request = self.request.clone();
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        self.open_job = Some(Job::spawn(
            &self.service,
            JobKind::HexCompare,
            title,
            &self.wake,
            move |svc, job| svc.hex_open(&request, job),
        ));
    }

    /// Aligns the opened documents in a job with the requested mode and the rule options; a running alignment is
    /// cancelled; the rows shown stay until the new alignment lands.
    fn align(&mut self) {
        let Some(opened) = self.opened.clone() else {
            return;
        };
        if let Some(job) = self.align_job.take() {
            job.cancel();
            self.counters.cancelled += 1;
        }
        self.stopped = false;
        self.counters.aligns += 1;
        let mut opts = self.align_options;
        opts.mode = self.mode;
        let aligner = Arc::clone(&self.aligner);
        let title = format!("Aligning {} ⇄ {}", self.names[0], self.names[1]);
        self.align_job = Some(Job::spawn(
            &self.service,
            JobKind::HexCompare,
            title,
            &self.wake,
            move |svc, job| {
                let alignment = aligner(svc, job, &opened, &opts)?;
                let raw_same = svc.hex_raw_same(&opened, job)?;
                let changed = svc.hex_changed(&opened);
                Ok(Landed {
                    alignment,
                    raw_same,
                    changed,
                })
            },
        ));
    }

    /// Polls the jobs (once per frame); true when something changed.
    pub fn poll(&mut self, _service: &Service) -> bool {
        let mut changed = self.screen.poll(self.generation);
        if let Some(result) = self.open_job.as_ref().and_then(Job::try_result) {
            self.open_job = None;
            changed = true;
            match result {
                Ok(opened) => self.opened_landed(opened),
                Err(ApiError::Cancelled) => {}
                Err(e) => {
                    let side = match &e {
                        ApiError::LoadFailed { side, .. } => Some(*side),
                        _ => None,
                    };
                    self.notes.push(LogNote {
                        kind: LogKind::Error,
                        text: e.to_string(),
                    });
                    self.error = Some((side, e.to_string()));
                    self.bump();
                }
            }
        }
        if let Some(result) = self.align_job.as_ref().and_then(Job::try_result) {
            self.align_job = None;
            changed = true;
            match result {
                Ok(landed) => self.land(landed),
                Err(ApiError::Cancelled) => self.stopped = true,
                Err(e) => {
                    self.notes.push(LogNote {
                        kind: LogKind::Error,
                        text: e.to_string(),
                    });
                    self.requests.push(HexRequest::Notice {
                        text: format!("The alignment failed: {e}"),
                        reload: false,
                    });
                    self.stopped = true;
                }
            }
        }
        changed |= self.poll_searches();
        changed |= self.poll_values();
        self.tick_primary();
        changed
    }

    fn opened_landed(&mut self, opened: HexOpened) {
        self.opened = Some(Arc::new(opened));
        self.alignment = None;
        self.response = None;
        self.stale = false;
        self.screen.clear();
        let max = self.columns().saturating_sub(1);
        for p in &mut self.panes {
            p.caret = p.caret.min(max);
            p.anchor = p.anchor.min(max);
        }
        self.rebuild(None);
        self.refresh_values();
        self.align();
    }

    /// The alignment landed: the rows are replaced in one frame, the caret's byte and the top byte kept.
    fn land(&mut self, landed: Landed) {
        let kept = self.kept();
        let Some(opened) = self.opened.clone() else {
            return;
        };
        let alignment = landed.alignment;
        self.ready_after = Some(self.started.elapsed());
        let response = self
            .service
            .hex_response(
                &opened,
                &alignment,
                self.display.layout,
                self.display.filter,
            )
            .with_raw_same(landed.raw_same);
        if alignment.degraded {
            self.requests.push(HexRequest::Notice {
                text: "The alignment ran out of its budget: some gaps are laid out byte by byte (degraded)."
                    .to_owned(),
                reload: false,
            });
        }
        for w in alignment
            .warnings
            .iter()
            .map(hikasa_session::api::hex_warning)
        {
            self.notes.push(LogNote {
                kind: LogKind::Warning,
                text: format!("[{}] {}", w.code.as_str(), w.message),
            });
        }
        self.stale = !landed.changed.is_empty();
        for w in &landed.changed {
            self.notes.push(LogNote {
                kind: LogKind::Warning,
                text: format!("[{}] {}", w.code.as_str(), w.message),
            });
            self.requests.push(HexRequest::Notice {
                text: format!(
                    "{} changed on disk during the comparison.",
                    w.spec.clone().unwrap_or_default()
                ),
                reload: true,
            });
        }
        self.alignment = Some(alignment);
        self.notes.push(LogNote {
            kind: LogKind::Summary,
            text: format!(
                "{}: {}",
                crate::table_view::verdict_text(response.kind),
                hikasa_report::hex_summary_line(&response.alignment.stats, response.alignment.mode)
            ),
        });
        self.response = Some(response);
        self.rebuild(Some((kept, true)));
        if let Some(step) = self.queued.take() {
            self.navigate(step);
        }
        self.refresh_values();
    }

    fn bump(&mut self) {
        self.generation += 1;
    }

    /// Rebuilds the source of the rows (a new generation) and puts the kept places back (`true`: a landing, by their
    /// bytes; else by their columns).
    fn rebuild(&mut self, kept: Option<(Kept, bool)>) {
        self.bump();
        self.source = self.opened.clone().map(|opened| {
            Source::new(
                opened,
                self.alignment.clone(),
                self.display.layout,
                self.display.filter,
                self.display.context,
            )
        });
        if let Some((kept, landed)) = kept {
            self.restore(kept, landed);
        }
    }

    /// The focus pane's caret and anchor, and the top row's first column, with their bytes.
    fn kept(&self) -> Kept {
        let pane = self.pane(self.focus);
        let none = Place {
            col: 0,
            sides: [(0, false); 2],
        };
        let Some(src) = &self.source else {
            return Kept {
                caret: none,
                anchor: none,
                top: None,
            };
        };
        let view = src.view();
        let store = view.segments();
        let place = |col: u64| {
            let (l, r) = store.offsets_at(col);
            let (nl, nr) = store.offsets_at(col + 1);
            Place {
                col,
                sides: [(l, nl > l), (r, nr > r)],
            }
        };
        let bpl = u64::from(view.layout().bytes_per_line.clamp(1, 256));
        Kept {
            caret: place(pane.caret),
            anchor: place(pane.anchor),
            top: view
                .unfiltered_row(self.screen.top)
                .map(|row| place(row * bpl)),
        }
    }

    /// Puts the kept places back. A filter or layout change keeps the columns (the rows change, the alignment does
    /// not); a landing maps each place by the focus side's byte, or the other side's byte at the same column when the
    /// focus side has none there (a gap, past its end).
    fn restore(&mut self, kept: Kept, landed: bool) {
        let Some(src) = self.source.clone() else {
            return;
        };
        let view = src.view();
        let store = view.segments();
        let last = store.columns().saturating_sub(1);
        let focus = self.focus;
        let col = |p: &Place| -> u64 {
            if !landed {
                return p.col.min(last);
            }
            let (mine, theirs) = (p.sides[index(focus)], p.sides[1 - index(focus)]);
            let (side, (offset, _)) = if mine.1 || !theirs.1 {
                (focus, mine)
            } else {
                (other(focus), theirs)
            };
            store
                .column_at_offset(side, offset)
                .unwrap_or(p.col.min(last))
        };
        // One column for both panes (rows are aligned).
        let (caret, anchor) = (col(&kept.caret), col(&kept.anchor));
        let top = kept.top.as_ref().map(col);
        drop(view);
        for p in &mut self.panes {
            p.caret = caret;
            p.anchor = anchor;
        }
        let row_count = self.row_count();
        if let Some(row) = top.and_then(|c| self.row_of_column(c)) {
            self.screen.set_top(row, row_count);
        }
        self.screen.clamp(row_count);
    }

    // ---- facts ---------------------------------------------------------------------------------------------------

    /// The request (with the mode the view runs).
    pub fn request(&self) -> &HexCompareRequest {
        &self.request
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&self.names[0], &self.names[1]],
        )
    }

    /// The opened documents.
    pub fn opened(&self) -> Option<&Arc<HexOpened>> {
        self.opened.as_ref()
    }

    /// The landed alignment.
    pub fn alignment(&self) -> Option<&Arc<HexAlignment>> {
        self.alignment.as_ref()
    }

    /// The landed comparison as `hex compare` builds it (Info, the outcome, the reports).
    pub fn response(&self) -> Option<&HexCompareResponse> {
        self.response.as_ref()
    }

    /// The rows' source this generation.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn source(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// The display options.
    pub fn display(&self) -> &HexDisplay {
        &self.display
    }

    /// The error of a failed open (the side when one failed).
    pub fn error(&self) -> Option<&(Option<Side>, String)> {
        self.error.as_ref()
    }

    /// The phase.
    pub fn phase(&self) -> HexPhase {
        if self.error.is_some() {
            HexPhase::Failed
        } else if self.is_empty() {
            HexPhase::Empty
        } else if self.opened.is_none() {
            HexPhase::Opening
        } else if self.alignment.is_some() {
            HexPhase::Aligned
        } else {
            HexPhase::Provisional
        }
    }

    /// The alignment was stopped (Esc) or failed: the rows stay provisional.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// An input changed during the last job (`input_changed`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stale(&self) -> bool {
        self.stale
    }

    /// The alignment job runs: its progress (phase, fraction).
    pub fn aligning(&self) -> Option<(Option<hikasa_core::Phase>, Option<f32>)> {
        self.align_job.as_ref().map(Job::progress)
    }

    /// The pane of a side.
    pub fn pane(&self, side: Side) -> &PaneState {
        &self.panes[index(side)]
    }

    /// The focus side.
    pub fn focus(&self) -> Side {
        self.focus
    }

    /// The current find hit.
    pub fn find_hit(&self) -> Option<&(Side, Range<u64>)> {
        self.hit.as_ref()
    }

    /// Aligned columns of the pair (0 before the documents open).
    pub fn columns(&self) -> u64 {
        self.source
            .as_ref()
            .map_or(0, |s| s.view().segments().columns())
    }

    /// Rows under the filter.
    pub fn row_count(&self) -> u64 {
        self.source.as_ref().map_or(0, |s| s.view().row_count())
    }

    /// Rows without a filter.
    pub fn full_row_count(&self) -> u64 {
        self.source
            .as_ref()
            .map_or(0, |s| s.view().full_row_count())
    }

    /// The filtered row showing column `col` (the next shown one when hidden).
    pub fn row_of_column(&self, col: u64) -> Option<u64> {
        let src = self.source.as_ref()?;
        let view = src.view();
        let bpl = u64::from(view.layout().bytes_per_line.clamp(1, 256));
        view.filtered_row(col / bpl)
            .or_else(|| view.row_count().checked_sub(1))
    }

    /// The first column of filtered row `k` (`None` for the marker row or past the end).
    pub fn first_column_of(&self, k: u64) -> Option<u64> {
        let src = self.source.as_ref()?;
        let view = src.view();
        let bpl = u64::from(view.layout().bytes_per_line.clamp(1, 256));
        view.unfiltered_row(k).map(|r| r * bpl)
    }

    /// The bytes of `side` at column `col` (its byte, or the gap's next byte) and whether the column holds one.
    pub fn byte_at(&self, side: Side, col: u64) -> Option<(u64, bool)> {
        let src = self.source.as_ref()?;
        let view = src.view();
        let store = view.segments();
        let (l, r) = store.offsets_at(col);
        let offset = if side == Side::Right { r } else { l };
        let (nl, nr) = store.offsets_at(col + 1);
        let next = if side == Side::Right { nr } else { nl };
        Some((offset, next > offset))
    }

    /// The selected columns of the focus pane.
    pub fn selection(&self) -> Range<u64> {
        self.pane(self.focus).selection()
    }

    // ---- frames --------------------------------------------------------------------------------------------------

    /// Prepares the rows of a frame showing `rows_visible` whole rows at `now` (the window's clock): the row cache,
    /// try_rows or a prefetch. True when every row is drawn from the cache.
    pub fn frame(&mut self, rows_visible: u32, now: f64) -> bool {
        self.now = now;
        self.screen.rows_visible = rows_visible.max(1);
        self.tick_primary();
        let Some(src) = self.source.clone() else {
            return true;
        };
        self.screen
            .prepare(&src, self.generation, &self.service, &self.wake)
    }

    // ---- caret, selection, pointer -------------------------------------------------------------------------------

    /// A caret key in the focus pane (`shift` extends from the anchor). PgUp/PgDn page the screen and keep the caret at
    /// the same place on it (W13a review F3). Under a filter a caret on a hidden row moves to the next shown row
    /// (Down, PgDn) or the previous one (Up, PgUp) (F4).
    pub fn key(&mut self, key: CaretKey, shift: bool) {
        let Some(src) = self.source.clone() else {
            return;
        };
        let view = src.view();
        let columns = view.segments().columns();
        if columns == 0 {
            return;
        }
        let bpl = u64::from(view.layout().bytes_per_line.clamp(1, 256));
        let caret = self.pane(self.focus).caret;
        let in_row = caret % bpl;
        let last = columns - 1;
        let rows = view.row_count();
        // The caret's shown row, or for a hidden row the next shown one (`false`), or none after it.
        let here = view
            .filtered_row(caret / bpl)
            .map(|f| (f, view.unfiltered_row(f) == Some(caret / bpl)));
        let row_step = |k: i64| -> u64 {
            // The same position in the k-th shown row from the caret's (under a filter: hidden rows skipped).
            if rows == 0 {
                return caret;
            }
            let k = i128::from(k);
            let target = match here {
                Some((f, true)) => i128::from(f) + k,
                // Hidden: Down reaches the next shown row (`f`), Up the previous one (`f - 1`).
                Some((f, false)) if k > 0 => i128::from(f) + k - 1,
                Some((f, false)) => i128::from(f) + k,
                None if k > 0 => return caret,
                None => i128::from(rows) + k,
            };
            match view.unfiltered_row(target.clamp(0, i128::from(rows - 1)) as u64) {
                Some(row) => (row * bpl + in_row).min(last),
                None => caret,
            }
        };
        let page = i64::from(self.screen.rows_visible.max(1));
        let to = match key {
            CaretKey::Left => caret.saturating_sub(1),
            CaretKey::Right => (caret + 1).min(last),
            CaretKey::Up => row_step(-1),
            CaretKey::Down => row_step(1),
            CaretKey::Home => caret - in_row,
            CaretKey::End => (caret - in_row + bpl - 1).min(last),
            CaretKey::First => 0,
            CaretKey::Last => last,
            CaretKey::PageUp => row_step(-page),
            CaretKey::PageDown => row_step(page),
        };
        drop(view);
        match key {
            CaretKey::PageUp | CaretKey::PageDown => {
                // The screen pages, the caret moves the same rows: it keeps its place on the screen (at the ends it
                // goes as far as it can and is kept in view).
                let n = if key == CaretKey::PageUp { -1 } else { 1 };
                self.screen.page(n, rows);
                self.place_caret(to, shift, false);
                if let Some(row) = self.row_of_column(to) {
                    self.screen.reveal(row, rows, false);
                }
            }
            _ => self.set_caret(to, shift),
        }
        if shift {
            self.primary_at = Some(self.now + crate::text::clipboard::PRIMARY_SETTLE_S);
        }
    }

    /// Moves the focus pane's caret to `col` (`extend`: the anchor stays), keeps it in view.
    pub fn set_caret(&mut self, col: u64, extend: bool) {
        self.place_caret(col, extend, true);
    }

    /// Moves the caret of both panes to `col` (`extend`: the anchors stay); `reveal`: scrolled into view.
    fn place_caret(&mut self, col: u64, extend: bool, reveal: bool) {
        let max = self.columns().saturating_sub(1);
        let col = col.min(max);
        // Both panes' carets are at the same column (rows are aligned).
        for p in &mut self.panes {
            p.caret = col;
            if !extend {
                p.anchor = col;
            }
        }
        if reveal && let Some(row) = self.row_of_column(col) {
            let rows = self.row_count();
            self.screen.reveal(row, rows, false);
        }
        self.refresh_values();
    }

    /// Tab: switches the caret's part (hex ↔ characters).
    pub fn switch_part(&mut self) {
        let shown = self.display.layout.chars != hikasa_hex::CharSet::None;
        for p in &mut self.panes {
            p.part = match p.part {
                Part::Hex if shown => Part::Chars,
                _ => Part::Hex,
            };
        }
    }

    /// Esc: collapses the selection; without one, cancels a running find, scan or alignment.
    pub fn escape(&mut self) {
        let k = index(self.focus);
        if self.panes[k].has_selection() {
            let caret = self.panes[k].caret;
            for p in &mut self.panes {
                p.anchor = caret;
            }
            return;
        }
        if self.cancel_searches() {
            return;
        }
        if let Some(job) = self.align_job.take() {
            job.cancel();
            self.counters.cancelled += 1;
            self.stopped = true;
            self.queued = None;
            self.requests
                .push(HexRequest::Message(Message::info("Alignment stopped.")));
        }
    }

    /// Select All: every column of the focus side.
    pub fn select_all(&mut self) {
        let columns = self.columns();
        if columns == 0 {
            return;
        }
        for p in &mut self.panes {
            p.anchor = 0;
            p.caret = columns - 1;
        }
        self.refresh_values();
        self.primary_at = Some(self.now);
    }

    /// The column of a hit (`None` on the address column or below the rows).
    pub fn hit_column(&self, hit: &Hit) -> Option<(u64, Part)> {
        let (part, i) = hit.cell?;
        let first = self.first_column_of(hit.row)?;
        let last = self.columns().checked_sub(1)?;
        Some(((first + u64::from(i)).min(last), part))
    }

    /// A click on a pane (`shift` extends): the caret on the hit cell, the pane focused; on an address, the row
    /// selected.
    pub fn click(&mut self, side: Side, hit: &Hit, shift: bool) {
        self.focus = side;
        self.dragging = Some(side);
        if hit.cell.is_none() {
            self.click_address(hit.row);
            return;
        }
        let Some((col, part)) = self.hit_column(hit) else {
            return;
        };
        for p in &mut self.panes {
            p.part = part;
        }
        self.set_caret(col, shift);
    }

    /// A drag to a hit (the button held): the selection extends.
    pub fn drag(&mut self, hit: &Hit) {
        if self.dragging.is_none() {
            return;
        }
        if let Some((col, _)) = self.hit_column(hit) {
            self.set_caret(col, true);
        }
    }

    /// The button was released: a selection settles (the primary selection).
    pub fn release(&mut self) {
        self.dragging = None;
        if self.pane(self.focus).has_selection() {
            self.primary_at = Some(self.now);
        }
    }

    /// A double-click: the group under the pointer is selected.
    pub fn double_click(&mut self, side: Side, hit: &Hit) {
        self.focus = side;
        let Some((col, _)) = self.hit_column(hit) else {
            return;
        };
        let g = u64::from(self.display.layout.group.max(1));
        let bpl = u64::from(self.display.layout.bytes_per_line.clamp(1, 256));
        let row0 = col - col % bpl;
        let start = row0 + (col - row0) / g * g;
        let end = (start + g).min(row0 + bpl).min(self.columns()) - 1;
        self.set_caret(start, false);
        self.set_caret(end, true);
        self.primary_at = Some(self.now);
    }

    /// A right-click: outside the selection the caret moves there first (then the window opens the menu).
    pub fn context_click(&mut self, side: Side, hit: &Hit) {
        self.focus = side;
        if let Some((col, _)) = self.hit_column(hit)
            && !self.selection().contains(&col)
        {
            self.set_caret(col, false);
        }
    }

    /// A click on an address: that row is selected.
    pub fn click_address(&mut self, row: u64) {
        let Some(first) = self.first_column_of(row) else {
            return;
        };
        let bpl = u64::from(self.display.layout.bytes_per_line.clamp(1, 256));
        let end = (first + bpl).min(self.columns()).saturating_sub(1);
        self.set_caret(first, false);
        self.set_caret(end, true);
        self.primary_at = Some(self.now);
    }

    /// The pane moves (`GoLeft`, `GoRight`, `GoNext`, `GoPrev`): the focus side changes, the column kept.
    pub fn focus_side(&mut self, side: Side) {
        self.focus = side;
        self.refresh_values();
    }

    fn tick_primary(&mut self) {
        let Some(at) = self.primary_at else {
            return;
        };
        if self.now + 1e-9 < at {
            return;
        }
        self.primary_at = None;
        if !self.pane(self.focus).has_selection() {
            return;
        }
        let chars = self.pane(self.focus).part == Part::Chars;
        match self.copy_now(chars, crate::text::clipboard::PRIMARY_MAX_BYTES as u64) {
            Some(Ok(text)) => {
                self.primary_written += 1;
                self.requests.push(HexRequest::Primary(text));
            }
            Some(Err(_)) => {}
            // Not cached: the bytes are read in a job, the primary selection written when it lands (W13a review F14).
            None => {
                if let Some(job) = self.primary_job.take() {
                    job.cancel();
                }
                self.primary_job =
                    self.copy_job(chars, crate::text::clipboard::PRIMARY_MAX_BYTES as u64);
            }
        }
    }

    // ---- navigation ----------------------------------------------------------------------------------------------

    /// A navigation step: with the alignment, `nav::step` (the section selected for a difference); past a truncated
    /// section list, and in mode None before the alignment lands, the differing run after (before) the caret's run in a
    /// scan job (`nav::scan_difference_run`, the run selected; W13a review F8); before the alignment lands the
    /// existing-byte steps on the provisional layout, a difference step in Fast or Complete queued until it lands.
    pub fn navigate(&mut self, step: Step) {
        let Some(src) = self.source.clone() else {
            return;
        };
        let caret = self.pane(self.focus).caret;
        let forward = step == Step::NextDifference;
        match (self.alignment.clone(), step) {
            (Some(al), Step::NextDifference | Step::PrevDifference) => {
                let truncated = al.sections.truncated();
                if truncated && !forward && caret >= al.sections.stored_end() {
                    // Runs past the stored list may lie between the caret and the last stored section.
                    self.scan(caret, false);
                    return;
                }
                match hikasa_hex::nav::step(&al, caret, step) {
                    Some(col) => self.select_section(&al, col),
                    None if truncated && forward => self.scan(caret, true),
                    None => self.no_more(),
                }
            }
            (Some(al), _) => match hikasa_hex::nav::step(&al, caret, step) {
                Some(col) => self.set_caret(col, false),
                None => self.no_more_bytes(),
            },
            (None, Step::NextDifference | Step::PrevDifference) => {
                if self.mode == AlignMode::None {
                    self.scan(caret, forward);
                } else if self.align_job.is_some() {
                    self.queued = Some(step);
                    self.requests.push(HexRequest::Message(Message::info(
                        "The step runs when the comparison is complete.",
                    )));
                } else {
                    self.no_more();
                }
            }
            (None, _) => {
                let view = src.view();
                match hikasa_hex::nav::step_store(view.segments(), caret, step) {
                    Some(col) => {
                        drop(view);
                        self.set_caret(col, false);
                    }
                    None => {
                        drop(view);
                        self.no_more_bytes();
                    }
                }
            }
        }
    }

    fn select_section(&mut self, al: &Arc<HexAlignment>, col: u64) {
        let i = al.sections.first_ending_after(col);
        let end = al
            .sections
            .columns(i)
            .filter(|c| c.start == col)
            .map_or(col + 1, |c| c.end);
        self.select_run(col..end);
    }

    /// A difference's columns selected, the caret on its first column (open default), scrolled to the upper third.
    fn select_run(&mut self, run: Range<u64>) {
        let (col, end) = (run.start, run.end.saturating_sub(1).max(run.start));
        for p in &mut self.panes {
            p.caret = col;
            p.anchor = end;
        }
        if let Some(row) = self.row_of_column(col) {
            let rows = self.row_count();
            self.screen.reveal(row, rows, true);
        }
        self.refresh_values();
        self.primary_at = Some(self.now);
    }

    fn no_more(&mut self) {
        self.requests
            .push(HexRequest::Message(Message::info(NO_MORE)));
    }

    fn no_more_bytes(&mut self) {
        self.requests.push(HexRequest::Message(Message::info(
            "No more bytes of this side.",
        )));
    }

    /// The scan for the differing run after (`forward`) or before the caret's run from `from` in a job
    /// (*Searching for the next difference…*; Esc cancels): the alignment's segments, or None's layout before it lands.
    fn scan(&mut self, from: u64, forward: bool) {
        let Some(opened) = self.opened.clone() else {
            return;
        };
        if let Some(job) = self.scan_job.take() {
            job.cancel();
        }
        let alignment = self.alignment.clone();
        let title = if forward {
            "Searching for the next difference…"
        } else {
            "Searching for the previous difference…"
        };
        self.scan_job = Some(Job::spawn(
            &self.service,
            JobKind::HexCompare,
            title.into(),
            &self.wake,
            move |_, job| {
                let (l, r) = (&*opened.documents.left, &*opened.documents.right);
                hikasa_hex::nav::scan_difference_run(
                    l,
                    r,
                    alignment.as_deref(),
                    from,
                    forward,
                    &job.ctl(),
                )
                .map_err(|e| {
                    if e.is_cancelled() {
                        ApiError::Cancelled
                    } else {
                        ApiError::InvalidArgument(e.to_string())
                    }
                })
            },
        ));
    }

    /// Go To (D-358): the core's resolved address on `side` (the dialog's): the caret on that byte; a clamp text is the
    /// message panel line.
    pub fn go_to(&mut self, side: Side, query: hikasa_hex::nav::AddressQuery) {
        let Some(opened) = self.opened.clone() else {
            return;
        };
        let doc = if side == Side::Right {
            &opened.documents.right
        } else {
            &opened.documents.left
        };
        let caret_offset = self
            .byte_at(side, self.pane(side).caret)
            .map_or(0, |(o, _)| o);
        let Some((offset, clamp)) = query.resolve(caret_offset, doc.len()) else {
            self.requests.push(HexRequest::Message(Message::info(
                "The file is empty: there is no byte to go to.",
            )));
            return;
        };
        self.focus = side;
        let col = self
            .source
            .as_ref()
            .and_then(|s| s.view().segments().column_at_offset(side, offset))
            .unwrap_or(0);
        self.set_caret(col, false);
        if let Some(row) = self.row_of_column(col) {
            let rows = self.row_count();
            self.screen.reveal(row, rows, true);
        }
        if let Some(text) = clamp {
            self.requests
                .push(HexRequest::Message(Message::info(capitalized(&text))));
        }
    }

    /// The thumbnail was clicked or dragged at `fraction` (0…1) of its height: the view scrolls to the column of that
    /// place (the nearest shown row under a filter), that row at the screen's middle.
    pub fn thumbnail_to(&mut self, fraction: f64) {
        let columns = self.columns();
        if columns == 0 {
            return;
        }
        let col = ((columns as f64) * fraction.clamp(0.0, 1.0)).floor() as u64;
        let col = col.min(columns - 1);
        if let Some(row) = self.row_of_column(col) {
            let rows = self.row_count();
            let half = u64::from(self.screen.rows_visible / 2);
            self.screen.set_top(row.saturating_sub(half), rows);
        }
    }

    // ---- find ------------------------------------------------------------------------------------------------------

    /// Find Next / Find Previous (D-359): `HexFinder::next` in a job from the caret's byte (+1 forward) on the
    /// panel's side; a newer search cancels the older.
    pub fn find_step(&mut self, forward: bool) {
        let Some(finder) = self.find.finder().cloned() else {
            if let Some(e) = self.find.message.clone() {
                self.requests.push(HexRequest::Message(Message::info(e)));
            }
            return;
        };
        let Some(opened) = self.opened.clone() else {
            return;
        };
        let side = self.find.side;
        let caret = self.pane(side).caret;
        let (at, held) = self.byte_at(side, caret).unwrap_or((0, false));
        // From a gap of the searched side the search starts at the gap's next byte (W13a review F10).
        let from = if forward && held { at + 1 } else { at };
        let dir = if forward {
            hikasa_hex::Direction::Forward
        } else {
            hikasa_hex::Direction::Backward
        };
        if let Some((_, _, _, job)) = self.find_job.take() {
            job.cancel();
        }
        let wrap = self.find.wrap;
        self.find.remember();
        self.find_job = Some((
            side,
            from,
            dir,
            Job::spawn(
                &self.service,
                JobKind::HexCompare,
                "Searching…".into(),
                &self.wake,
                move |svc, job| {
                    let doc = if side == Side::Right {
                        &opened.documents.right
                    } else {
                        &opened.documents.left
                    };
                    finder
                        .next(doc, from, dir, wrap, &svc.stream_options(), &job.ctl())
                        .map_err(|e| {
                            if e.is_cancelled() {
                                ApiError::Cancelled
                            } else {
                                ApiError::InvalidArgument(e.to_string())
                            }
                        })
                },
            ),
        ));
    }

    /// The find job runs: its progress fraction.
    pub fn searching(&self) -> Option<Option<f32>> {
        self.find_job.as_ref().map(|(.., j)| j.progress().1)
    }

    /// Cancels a running find or scan; true when one ran.
    pub fn cancel_searches(&mut self) -> bool {
        let mut any = false;
        if let Some((.., job)) = self.find_job.take() {
            job.cancel();
            any = true;
        }
        if let Some(job) = self.scan_job.take() {
            job.cancel();
            any = true;
        }
        any
    }

    fn poll_searches(&mut self) -> bool {
        let mut changed = false;
        if let Some(result) = self.scan_job.as_ref().and_then(Job::try_result) {
            self.scan_job = None;
            changed = true;
            match result {
                Ok(Some(run)) => self.select_run(run),
                Ok(None) => self.no_more(),
                Err(ApiError::Cancelled) => {}
                Err(e) => self
                    .requests
                    .push(HexRequest::Message(Message::error(e.to_string()))),
            }
        }
        if let Some(result) = self.find_job.as_ref().and_then(|(.., j)| j.try_result()) {
            let Some((side, from, dir, _)) = self.find_job.take() else {
                return changed;
            };
            changed = true;
            match result {
                Ok(Some(range)) => {
                    let wrapped = match dir {
                        hikasa_hex::Direction::Forward => range.start < from,
                        hikasa_hex::Direction::Backward => range.start >= from,
                    };
                    self.show_hit(side, range);
                    if wrapped {
                        let text = if dir == hikasa_hex::Direction::Forward {
                            "Wrapped to the start."
                        } else {
                            "Wrapped to the end."
                        };
                        self.requests.push(HexRequest::Message(Message::info(text)));
                    }
                }
                Ok(None) => {
                    self.hit = None;
                    self.requests
                        .push(HexRequest::Message(Message::info("Not found.")));
                }
                Err(ApiError::Cancelled) => {}
                Err(e) => self
                    .requests
                    .push(HexRequest::Message(Message::error(e.to_string()))),
            }
        }
        changed
    }

    /// A hit: its bytes selected on that side, the caret at its start, scrolled into view.
    fn show_hit(&mut self, side: Side, range: Range<u64>) {
        let Some(src) = self.source.clone() else {
            return;
        };
        let view = src.view();
        let store = view.segments();
        let (Some(start), Some(end)) = (
            store.column_at_offset(side, range.start),
            store.column_at_offset(side, range.end.saturating_sub(1)),
        ) else {
            return;
        };
        drop(view);
        self.focus = side;
        self.hit = Some((side, range));
        for p in &mut self.panes {
            p.anchor = end;
            p.caret = start;
        }
        if let Some(row) = self.row_of_column(start) {
            let rows = self.row_count();
            self.screen.reveal(row, rows, true);
        }
        self.refresh_values();
    }

    // ---- values ---------------------------------------------------------------------------------------------------

    /// The value panel's lines and whether they are stale (a job reads the bytes).
    pub fn value_lines(&self) -> Option<&(String, bool)> {
        self.value_lines.as_ref()
    }

    /// Recomputes the value panel at the caret's byte of the focus side (a gap: its next byte): from the page cache on
    /// the UI thread (`values::at_cached`, never a read), else in a job; the panel keeps its previous lines greyed
    /// until it lands. A running job of an earlier byte is cancelled on both paths (W13a review F7).
    fn refresh_values(&mut self) {
        self.drop_values_job();
        if self.display.values == HexValues::None {
            return;
        }
        let Some(opened) = self.opened.clone() else {
            return;
        };
        let side = self.focus;
        let Some((offset, _)) = self.byte_at(side, self.pane(side).caret) else {
            return;
        };
        let doc = if side == Side::Right {
            Arc::clone(&opened.documents.right)
        } else {
            Arc::clone(&opened.documents.left)
        };
        let fmt = self.value_format();
        if let Some(values) = hikasa_hex::values::at_cached(&doc, offset) {
            self.value_lines = Some((values_text(values, fmt), false));
            return;
        }
        if let Some(lines) = &mut self.value_lines {
            lines.1 = true;
        }
        self.values_job = Some((
            (side, offset),
            Job::spawn(
                &self.service,
                JobKind::VfsRead,
                "read the values".into(),
                &self.wake,
                move |_, _| {
                    hikasa_hex::values::at(&doc, offset)
                        .map_err(|e| ApiError::InvalidArgument(e.to_string()))
                },
            ),
        ));
    }

    /// Cancels and forgets the value panel's job.
    fn drop_values_job(&mut self) {
        if let Some((_, job)) = self.values_job.take() {
            job.cancel();
            self.counters.values_cancelled += 1;
        }
    }

    fn value_format(&self) -> hikasa_hex::ValueFormat {
        hikasa_hex::ValueFormat {
            little: self.display.values.little(),
            big: self.display.values.big(),
            radix: self.display.values_radix,
        }
    }

    /// The value panel's job landed: its lines when it read the caret's byte now (another byte's answer is dropped
    /// and the panel recomputed); a read error is the panel's text.
    fn poll_values(&mut self) -> bool {
        let Some(result) = self.values_job.as_ref().and_then(|(_, j)| j.try_result()) else {
            return false;
        };
        let Some((read, _)) = self.values_job.take() else {
            return false;
        };
        let side = self.focus;
        let now = self
            .byte_at(side, self.pane(side).caret)
            .map(|(offset, _)| (side, offset));
        if now != Some(read) || self.display.values == HexValues::None {
            self.refresh_values();
            return true;
        }
        match result {
            Ok(v) => {
                self.value_lines = Some((
                    hikasa_hex::values::format_with(&v, self.value_format()),
                    false,
                ));
            }
            Err(ApiError::Cancelled) => {}
            Err(e) => {
                self.value_lines = Some((format!("The values could not be read: {e}"), false));
            }
        }
        true
    }

    // ---- copy ------------------------------------------------------------------------------------------------------

    /// The copy of the focus side's bytes in the selection from the page cache (`copy::selection_text_cached`, never a
    /// read; `None`: not cached, the copy runs in a job), as hex pairs (`chars`: false, from the hex part) or
    /// characters.
    fn copy_now(&self, chars: bool, limit: u64) -> Option<Result<String, String>> {
        let src = self.source.as_ref()?;
        let side = self.focus;
        let doc = if side == Side::Right {
            &src.opened.documents.right
        } else {
            &src.opened.documents.left
        };
        let view = src.view();
        hikasa_hex::copy::selection_text_cached(
            doc,
            view.segments(),
            self.selection(),
            side,
            copy::form(chars, self.display.layout.chars),
            limit,
        )
        .map(|r| r.map_err(|e| e.to_string()))
    }

    /// The copy of the selection in a job (its bytes are not cached), at most `limit` bytes.
    fn copy_job(&self, chars: bool, limit: u64) -> Option<Job<String>> {
        let src = self.source.clone()?;
        let (side, columns) = (self.focus, self.selection());
        let form = copy::form(chars, self.display.layout.chars);
        Some(Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy the bytes".into(),
            &self.wake,
            move |_, _| copy::copy_in_job(&src, side, columns, form, limit),
        ))
    }

    /// Copy (`as_text`: Copy as Text, always the characters): the focus side's selected bytes as text, at most
    /// 16 MiB; in a job when the bytes are not cached.
    pub fn copy(&mut self, as_text: bool) {
        let chars = as_text || self.pane(self.focus).part == Part::Chars;
        let what = if chars { "characters" } else { "bytes" };
        match self.copy_now(chars, hikasa_hex::copy::COPY_LIMIT) {
            Some(Ok(text)) => self.requests.push(HexRequest::Clipboard(
                text,
                format!("The selected {what} were copied."),
            )),
            Some(Err(e)) => self.requests.push(HexRequest::Message(Message::warning(e))),
            None => {
                if let Some(job) = self.copy_job.take() {
                    job.cancel();
                }
                self.copy_job = self.copy_job(chars, hikasa_hex::copy::COPY_LIMIT);
            }
        }
    }

    /// Polls the copy jobs: the text for the window's clipboard; the primary selection's.
    pub fn poll_copy(&mut self) -> bool {
        if let Some(result) = self.primary_job.as_ref().and_then(Job::try_result) {
            self.primary_job = None;
            if let Ok(text) = result {
                self.primary_written += 1;
                self.requests.push(HexRequest::Primary(text));
            }
        }
        let Some(result) = self.copy_job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.copy_job = None;
        match result {
            Ok(text) => self.requests.push(HexRequest::Clipboard(
                text,
                "The selection was copied.".to_owned(),
            )),
            Err(e) => self
                .requests
                .push(HexRequest::Message(Message::warning(e.to_string()))),
        }
        true
    }

    /// Current Byte Address (HX-007): the caret's byte of the focus side as `nav::address_text` writes it in the
    /// layout's radix, copied.
    pub fn copy_address(&mut self) {
        // The bare address (the text Go To reads back); *(gap)* is the status bar's (W13a review F9).
        let Some((text, _)) = self.address_text_of(self.focus) else {
            return;
        };
        self.requests.push(HexRequest::Clipboard(
            text.clone(),
            format!("The address {text} was copied."),
        ));
    }

    /// The address text of the caret's byte on `side` (`(gap)` marked on a gap column: the status bar's).
    pub fn address_of(&self, side: Side) -> Option<String> {
        let (text, held) = self.address_text_of(side)?;
        Some(if held { text } else { format!("{text} (gap)") })
    }

    /// `nav::address_text` of the caret's byte on `side` (on a gap column, the side's next byte) and whether the
    /// column holds it.
    fn address_text_of(&self, side: Side) -> Option<(String, bool)> {
        let src = self.source.as_ref()?;
        let view = src.view();
        let (offset, held) = self.byte_at(side, self.pane(self.focus).caret)?;
        let text = hikasa_hex::nav::address_text(
            side,
            offset,
            self.display.layout.address,
            view.address_width(),
        );
        Some((text, held))
    }

    // ---- commands --------------------------------------------------------------------------------------------------

    /// Fills the command context of this view.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.busy = self.busy();
        ctx.ready = self.opened.is_some();
        ctx.hex_landed = self.alignment.is_some();
        ctx.byte_selection = self.opened.is_some();
        ctx.focus_side = Some(self.focus);
        ctx.parents_available = true;
        ctx.next_difference = self.opened.is_some();
        ctx.prev_difference = self.opened.is_some();
        ctx.hex_mode = Some(self.mode);
        // The file manager's entries work on local files only (FC-063).
        if let Some(o) = &self.opened {
            ctx.explorer = [
                matches!(o.specs.left, hikasa_core::Spec::Local(_)),
                matches!(o.specs.right, hikasa_core::Spec::Local(_)),
            ];
        }
        ctx.hex_values = Some((self.display.values, self.display.values_radix));
    }

    /// The check state of a command (`None`: not a check or radio command of this view).
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        let d = &self.display;
        Some(match id {
            CommandId::FavCompareAll => d.filter == DisplayFilter::All,
            CommandId::FavDiff => d.filter == DisplayFilter::Differences,
            CommandId::FavSame => d.filter == DisplayFilter::Same,
            CommandId::ShowThumbnail => d.thumbnail,
            CommandId::ShowSpecInfo => d.headers,
            CommandId::SideBySide => d.panes == PaneLayout::SideBySide,
            CommandId::OverUnder => d.panes == PaneLayout::OverUnder,
            CommandId::LittleEndianList => d.values.little(),
            CommandId::BigEndianList => d.values.big(),
            _ => return None,
        })
    }

    /// Runs a view command; true when it was this view's.
    pub fn execute(&mut self, _service: &Service, id: CommandId) -> bool {
        match id {
            CommandId::Reload => self.reload(),
            CommandId::DiffItemNext => self.navigate(Step::NextDifference),
            CommandId::DiffItemPrev => self.navigate(Step::PrevDifference),
            CommandId::ExistsNext => self.navigate(Step::NextExisting(self.focus)),
            CommandId::ExistsPrev => self.navigate(Step::PrevExisting(self.focus)),
            CommandId::FavCompareAll => self.set_filter(DisplayFilter::All),
            CommandId::FavDiff => self.set_filter(DisplayFilter::Differences),
            CommandId::FavSame => self.set_filter(DisplayFilter::Same),
            CommandId::SelectAll => self.select_all(),
            CommandId::Copy => self.copy(false),
            CommandId::CopyAsText => self.copy(true),
            CommandId::CurrentAddress => self.copy_address(),
            CommandId::ShowThumbnail => self.display.thumbnail = !self.display.thumbnail,
            CommandId::ShowSpecInfo => self.display.headers = !self.display.headers,
            CommandId::SideBySide => self.display.panes = PaneLayout::SideBySide,
            CommandId::OverUnder => self.display.panes = PaneLayout::OverUnder,
            CommandId::DisplayFontIncrease => self.set_font(self.display.font_px + 1),
            CommandId::DisplayFontDecrease => {
                self.set_font(self.display.font_px.saturating_sub(1));
            }
            CommandId::DisplayFontReset => self.set_font(self.default_font_px),
            CommandId::GoLeft => self.focus_side(Side::Left),
            CommandId::GoRight => self.focus_side(Side::Right),
            CommandId::GoNext | CommandId::GoPrev => self.focus_side(other(self.focus)),
            CommandId::Find => self.open_find(),
            CommandId::FindNext => self.find_step(true),
            CommandId::FindPrev => self.find_step(false),
            CommandId::LittleEndianList => self.set_values(self.display.values.toggled(true)),
            CommandId::BigEndianList => self.set_values(self.display.values.toggled(false)),
            _ => return false,
        }
        true
    }

    /// Opens the Find panel with the selection's bytes as its hex text (≤ 64 bytes).
    pub fn open_find(&mut self) {
        self.find.open = true;
        self.find.side = self.focus;
        let sel = self.selection();
        if sel.end - sel.start > 1
            && sel.end - sel.start <= 64
            && let Some(Ok(text)) = self.copy_now(false, 64)
            && !text.is_empty()
        {
            self.find.pattern = text;
            self.find.hex = true;
        }
        self.find.validate(&self.service);
    }

    /// The alignment mode switch (`AlignModeList`, the Alignment tab): a new alignment of the opened documents, the
    /// running one cancelled, the rows kept until it lands. The session's rule changes (dirty, D-84).
    pub fn set_mode(&mut self, mode: AlignMode) {
        self.mode = mode;
        self.request.rules.alignment = Some(mode.as_str().to_owned());
        self.queued = None;
        self.align();
    }

    /// The view runs another request (a rule tab's OK, D-370): a spec, a format or `unlocked_max` reopens; the other
    /// rules realign the opened documents; the layout words are the view's.
    pub fn relive(&mut self, request: HexCompareRequest) {
        let reopen = request.left != self.request.left
            || request.right != self.request.right
            || request.format != self.request.format
            || request.formats != self.request.formats
            || request.format_overrides != self.request.format_overrides
            || request.rules.unlocked_max_mib != self.request.rules.unlocked_max_mib;
        let realign = request.rules != self.request.rules;
        // The names follow the specs (Swap Sides, Next Difference Files, the Specs tab; W13a review F6): a side keeps
        // its title while its spec stays, takes the other's when the sides were swapped, else loses it.
        let old = [self.request.left.clone(), self.request.right.clone()];
        let new = [request.left.clone(), request.right.clone()];
        if new != old {
            let titles = self.titles.clone();
            self.titles = [0, 1].map(|k| {
                if new[k] == old[k] {
                    titles[k].clone()
                } else if new[k] == old[1 - k] {
                    titles[1 - k].clone()
                } else {
                    None
                }
            });
            self.names = [display_name(&new[0]), display_name(&new[1])];
        }
        self.request = request;
        match self.service.hex_settings(&self.request) {
            Ok(s) => {
                self.align_options = s.align;
                self.mode = s.align.mode;
            }
            Err(e) => {
                self.requests
                    .push(HexRequest::Message(Message::error(e.to_string())));
                return;
            }
        }
        if reopen {
            self.open();
        } else if realign {
            self.align();
        }
    }

    /// Replaces the specs of one side or both (a drop, a header's path, Open) and reopens.
    pub fn replace_specs(
        &mut self,
        left: Option<std::ffi::OsString>,
        right: Option<std::ffi::OsString>,
    ) {
        let mut request = self.request.clone();
        if let Some(l) = left {
            request.left = l;
        }
        if let Some(r) = right {
            request.right = r;
        }
        self.relive(request);
    }

    /// The format of one side (`None`: *Auto*; the header's picker, `FileFormat`): that side reopens.
    pub fn set_format(&mut self, side: Side, format: Option<String>) {
        let mut request = self.request.clone();
        match side {
            Side::Right => request.formats.right = format,
            _ => request.formats.left = format,
        }
        self.relive(request);
    }

    /// Reload (F5): reopens and realigns.
    pub fn reload(&mut self) {
        self.screen.clear();
        self.open();
    }

    /// Sets the display filter: a new generation, the caret's column and the top row's column kept.
    pub fn set_filter(&mut self, filter: DisplayFilter) {
        if self.alignment.is_none() && filter != DisplayFilter::All {
            self.requests
                .push(HexRequest::Message(Message::info(FILTER_LATER)));
        }
        let kept = self.kept();
        self.display.filter = filter;
        self.rebuild(Some((kept, false)));
    }

    /// Applies the Layout dialog: a new generation, the caret's column and the top row's column kept.
    pub fn set_layout(
        &mut self,
        layout: HexLayout,
        addresses: bool,
        panes: PaneLayout,
        context: u8,
    ) {
        let kept = self.kept();
        self.display.layout = layout;
        self.display.addresses = addresses;
        self.display.panes = panes;
        self.display.context = context.min(64);
        if layout.chars == hikasa_hex::CharSet::None {
            for p in &mut self.panes {
                p.part = Part::Hex;
            }
        }
        self.rebuild(Some((kept, false)));
    }

    /// Takes a duplicated view's display (`TabDuplicate`): every option, the font and the splitter included; the rows
    /// (layout, filter, context) come with the request.
    pub fn keep_display(&mut self, display: &HexDisplay) {
        let rows = (
            self.display.layout,
            self.display.filter,
            self.display.context,
        );
        self.display = *display;
        (
            self.display.layout,
            self.display.filter,
            self.display.context,
        ) = rows;
        self.refresh_values();
    }

    /// The value panel's blocks.
    pub fn set_values(&mut self, values: HexValues) {
        self.display.values = values;
        self.refresh_values();
        if values == HexValues::None {
            self.value_lines = None;
        } else if let Some((text, stale)) = &self.value_lines {
            // The blocks shown change at once when the bytes are cached (else the job's answer formats them).
            let _ = (text, stale);
        }
    }

    /// The value panel's integer radix.
    pub fn set_values_radix(&mut self, radix: Radix) {
        self.display.values_radix = radix;
        self.refresh_values();
    }

    /// The help topic F1 shows: the panel with the keyboard (Find, values), else the view's.
    pub fn help_topic(&self) -> &'static str {
        self.panel
            .unwrap_or(crate::commands::Scope::Hex.help_topic())
    }

    /// Sets the enabled Hex formats' names (the window, at build).
    pub fn set_format_names(&mut self, names: Vec<String>) {
        self.format_names = names;
    }

    /// The enabled Hex formats' names.
    pub fn format_names(&self) -> &[String] {
        &self.format_names
    }

    /// The File Info headers are shown.
    pub fn show_headers(&self) -> bool {
        self.display.headers
    }

    /// The panes' horizontal scroll (both together).
    pub fn set_scroll_x(&mut self, x: f32) {
        for p in &mut self.panes {
            p.scroll_x = x;
        }
    }

    /// Digits of an address of the larger file (core's `address_width`).
    pub fn address_width(&self) -> u8 {
        self.source.as_ref().map_or(8, |s| s.view().address_width())
    }

    /// The facts of an opened side for its header (core's `HexSide`).
    pub fn header_facts(&self, side: Side) -> Option<crate::file_header::HeaderFacts> {
        let opened = self.opened.as_ref()?;
        let s = if side == Side::Right {
            &opened.sides.right
        } else {
            &opened.sides.left
        };
        let zone = self.service.clock().now().time_zone().clone();
        Some(crate::file_header::hex_facts(s, &zone))
    }

    /// Compiles the Find panel's pattern after an edit (the core's message on a refusal).
    pub fn validate_find(&mut self) {
        let service = self.service.clone();
        self.find.validate(&service);
    }

    /// The font size of the display font dialog.
    pub fn set_font_px(&mut self, px: u32) {
        self.set_font(px);
    }

    /// Clear Session (D-243): the view becomes an empty view of its type, the headers take the specs.
    pub fn clear(&mut self) {
        self.cancel();
        // Nothing of the old pair survives: a late job, a queued step, the stale flag, the hit (W13a review F15).
        self.drop_values_job();
        self.copy_job = None;
        self.primary_job = None;
        self.queued = None;
        self.stale = false;
        self.stopped = false;
        self.hit = None;
        self.value_lines = None;
        self.primary_at = None;
        self.request.left = std::ffi::OsString::new();
        self.request.right = std::ffi::OsString::new();
        self.request.formats = Default::default();
        self.names = [String::new(), String::new()];
        self.titles = [None, None];
        self.opened = None;
        self.alignment = None;
        self.response = None;
        self.error = None;
        self.open_job = None;
        self.align_job = None;
        self.screen.clear();
        self.rebuild(None);
    }

    /// True for a view without its two specs (Clear Session, New Session ▸ Hex Compare): the headers take them.
    pub fn is_empty(&self) -> bool {
        self.request.left.is_empty() || self.request.right.is_empty()
    }

    fn set_font(&mut self, px: u32) {
        self.display.font_px = px.clamp(FONT_RANGE.0, FONT_RANGE.1);
    }

    // ---- state, sessions, jobs ------------------------------------------------------------------------------------

    /// Sets what the view may write (FC-003; a hex view writes nothing).
    pub fn set_write_policy(&mut self, write: crate::folder_view::WritePolicy) {
        self.write = write;
    }

    /// What the view may write.
    pub fn write_policy(&self) -> crate::folder_view::WritePolicy {
        self.write
    }

    /// Applies the layout of the view type and a session's `[view]` keys (D-108, D-370): `font_size` (else the theme's
    /// editor size), `layout`, `splitter`, `addresses`, `thumbnail`, `values`, `values_radix`. A value of the wrong type
    /// is ignored.
    pub fn apply_layout(&mut self, layout: &State, default_px: u32) {
        self.default_font_px = default_px.clamp(FONT_RANGE.0, FONT_RANGE.1);
        let value = |key: &str| layout.layout_value(VIEW, key);
        let size = value("font_size")
            .and_then(LayoutValue::as_float)
            .map_or(self.default_font_px, |f| {
                f.round().clamp(1.0, 1000.0) as u32
            });
        self.set_font(size);
        let flag = |key: &str| value(key).and_then(LayoutValue::as_bool);
        if let Some(on) = flag("thumbnail") {
            self.display.thumbnail = on;
        }
        if let Some(on) = flag("addresses") {
            self.display.addresses = on;
        }
        if let Some(LayoutValue::Text(word)) = value("layout") {
            self.display.panes = if word == PaneLayout::OverUnder.word() {
                PaneLayout::OverUnder
            } else {
                PaneLayout::SideBySide
            };
        }
        if let Some(f) = value("splitter").and_then(LayoutValue::as_float) {
            self.set_split(f as f32);
        }
        if let Some(LayoutValue::Text(word)) = value("values")
            && let Ok(v) = HexValues::parse(&word)
        {
            self.display.values = v;
        }
        if let Some(LayoutValue::Text(word)) = value("values_radix")
            && let Some(r) = radix_of(&word)
        {
            self.display.values_radix = r;
        }
    }

    /// The layout keys of the view type (`[layout.hex-compare]`): what every Hex Compare view shares.
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            (
                "font_size",
                LayoutValue::Float(f64::from(self.display.font_px)),
            ),
            (
                "layout",
                LayoutValue::Text(self.display.panes.word().to_owned()),
            ),
            (
                "splitter",
                LayoutValue::Float((f64::from(self.display.split) * 100.0).round() / 100.0),
            ),
        ]
    }

    /// Sets the splitter (a drag; clamped to [`SPLIT_RANGE`]; a value that is no number is ignored).
    pub fn set_split(&mut self, split: f32) {
        if split.is_finite() {
            self.display.split = split.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
        }
    }

    /// The view state a session keeps in `[view]` (D-349, D-370; never dirty), written by core
    /// (`hex_view_table`).
    pub fn session_table(&self) -> hikasa_session::sessions::OpaqueTable {
        let d = &self.display;
        hikasa_session::api::hex_view_table(
            &d.layout,
            d.panes == PaneLayout::OverUnder,
            d.context,
            &hikasa_session::api::HexViewState {
                addresses: d.addresses,
                thumbnail: d.thumbnail,
                values: d.values,
                values_radix: d.values_radix,
            },
        )
    }

    /// The `[display] filter` word.
    pub fn display_word(&self) -> &'static str {
        self.display.filter.as_str()
    }

    /// The inputs watched for changes on disk (D-118).
    pub fn inputs(&self) -> [String; 2] {
        [lossy(&self.request.left), lossy(&self.request.right)]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, sessions): the mode, the layout words and the
    /// filter of the view.
    pub fn duplicate(&self) -> (ViewRequest, [Option<String>; 2]) {
        let mut request = self.request.clone();
        let l = &self.display.layout;
        request.layout = hikasa_session::api::HexLayoutWords {
            bytes_per_line: Some(l.bytes_per_line.to_string()),
            group: Some(l.group.to_string()),
            chars: Some(l.chars.as_str().to_owned()),
            address: Some(l.address.as_str().to_owned()),
            context: Some(self.display.context.to_string()),
            display: Some(self.display.filter.as_str().to_owned()),
        };
        (
            ViewRequest::Hex {
                request,
                layout: self.display.layout,
                filter: Some(self.display.filter),
            },
            self.titles.clone(),
        )
    }

    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.open_job.is_some()
            || self.align_job.is_some()
            || self.scan_job.is_some()
            || self.find_job.is_some()
            || self.copy_job.is_some()
            || self.primary_job.is_some()
            || self.values_job.is_some()
            || self.screen.busy()
    }

    /// True while a job has ended but its result is not published yet.
    pub fn settling(&self) -> bool {
        self.open_job.as_ref().is_some_and(Job::settling)
            || self.align_job.as_ref().is_some_and(Job::settling)
            || self.scan_job.as_ref().is_some_and(Job::settling)
            || self.find_job.as_ref().is_some_and(|(.., j)| j.settling())
            || self.copy_job.as_ref().is_some_and(Job::settling)
            || self.primary_job.as_ref().is_some_and(Job::settling)
            || self.values_job.as_ref().is_some_and(|(_, j)| j.settling())
            || self.screen.settling()
    }

    /// Stops every job (the view is closing).
    pub fn cancel(&mut self) {
        for job in [self.align_job.as_ref()].into_iter().flatten() {
            job.cancel();
        }
        if let Some(job) = &self.open_job {
            job.cancel();
        }
        self.cancel_searches();
        if let Some(job) = &self.copy_job {
            job.cancel();
        }
        if let Some(job) = &self.primary_job {
            job.cancel();
        }
        if let Some((_, job)) = &self.values_job {
            job.cancel();
        }
        self.screen.cancel();
    }

    /// The running jobs (the job indicator).
    pub fn jobs(&self, view: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
        let status = |title: &str, progress: (Option<hikasa_core::Phase>, Option<f32>)| JobStatus {
            view,
            title: title.to_owned(),
            phase: progress.0,
            fraction: progress.1,
        };
        let mut out = Vec::new();
        if let Some(j) = &self.open_job {
            out.push(status(j.title(), j.progress()));
        }
        if let Some(j) = &self.align_job {
            out.push(status(j.title(), j.progress()));
        }
        if let Some(j) = &self.scan_job {
            out.push(status(j.title(), j.progress()));
        }
        if let Some((.., j)) = &self.find_job {
            out.push(status(j.title(), j.progress()));
        }
        out
    }

    /// The notes for the log since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    /// What the view asks of its window since the last call.
    pub fn take_requests(&mut self) -> Vec<HexRequest> {
        self.poll_copy();
        std::mem::take(&mut self.requests)
    }

    /// The verdict landed.
    pub fn verdict(&self) -> Option<ContentKind> {
        self.response.as_ref().map(|r| r.kind)
    }

    /// The D-23 outcome when the view closes (D-331's codes 1, 11, 2, 13): the landed verdict; a running alignment is
    /// waited for at most 1 s; `LoadFailed` (100) for a side that could not be opened; else `NoVerdict`.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        let began = Instant::now();
        while self.response.is_none()
            && (self.align_job.is_some() || self.open_job.is_some())
            && began.elapsed() < CLOSE_WAIT
        {
            self.poll(service);
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(r) = &self.response {
            ViewOutcome::Text {
                result: content_result(r.kind),
                ignore_unimportant: false,
            }
        } else if self.error.is_some() {
            ViewOutcome::LoadFailed
        } else {
            ViewOutcome::NoVerdict
        }
    }

    /// Replaces the aligner (tests: a held gate).
    #[cfg(test)]
    pub(crate) fn set_aligner_for_test(&mut self, aligner: Aligner) {
        self.aligner = aligner;
    }
}

impl Drop for HexViewModel {
    /// A view that closes or is replaced stops its jobs.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The result of a content kind (hex has no error result here: a failed view is `LoadFailed`).
fn content_result(kind: ContentKind) -> ContentResult {
    match kind {
        ContentKind::BinarySame => ContentResult::BinarySame,
        ContentKind::RulesSame => ContentResult::RulesSame,
        ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
        ContentKind::ImportantDifferences => ContentResult::ImportantDifferences,
        _ => ContentResult::BinaryDifferent,
    }
}

/// The value panel's text of a cached read: the lines, or the read's error.
fn values_text(
    values: Result<hikasa_hex::Values, hikasa_hex::HexError>,
    fmt: hikasa_hex::ValueFormat,
) -> String {
    match values {
        Ok(v) => hikasa_hex::values::format_with(&v, fmt),
        Err(e) => format!("The values could not be read: {e}"),
    }
}

/// The radix whose core word is `word` (as the view wrote it to `state.toml` or a session's `[view]`: core's
/// `Radix::as_str`; another word is no radix).
pub fn radix_of(word: &str) -> Option<Radix> {
    Radix::ALL.iter().copied().find(|r| r.as_str() == word)
}

/// The index of a side in `[left, right]`.
pub fn index(side: Side) -> usize {
    usize::from(side == Side::Right)
}

/// The other side.
pub fn other(side: Side) -> Side {
    if side == Side::Right {
        Side::Left
    } else {
        Side::Right
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => {
            let mut s: String = c.to_uppercase().collect();
            s.push_str(chars.as_str());
            if !s.ends_with('.') {
                s.push('.');
            }
            s
        }
        None => String::new(),
    }
}

fn display_name(spec: &std::ffi::OsStr) -> String {
    crate::shared::spec_name(&lossy(spec)).to_owned()
}

fn lossy(spec: &std::ffi::OsStr) -> String {
    spec.to_string_lossy().into_owned()
}
