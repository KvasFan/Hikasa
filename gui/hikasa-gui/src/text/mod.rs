//! The Text Compare view model (docs/05 §4–§6; M2 contract §1.11.2–§1.11.13): a **live** comparison the panes edit
//! (`Service::text_compare_live`, D-294), one caret and selection per pane, the recompare scheduling, the save flow,
//! the File Info header and the status bar, the display filter and the *visible slice* the panes draw
//! (ARCH-GUI-002: only the rows on screen are laid out; nothing exists per line of the documents).
//!
//! First screen (ARCH-GUI-003): `CompareObserver::loaded` hands over both documents before the alignment starts, so
//! the panes show the text unaligned at once; the live comparison then replaces them with aligned, coloured rows.
//!
//! Every file of this module is toolkit-free (architecture check 7): every text, edit, step, row and byte comes from
//! core (`EditableDocument`, `LiveComparison`, `edit::commands`, `edit::cursor`, `nav`, a `Service` call); the module
//! adds geometry ([`wrap`], [`pane`]), input and the order of the jobs and dialogs. `EditableDocument::apply`,
//! `undo` and `redo` are called from [`edit`] only.

pub mod clipboard;
pub mod edit;
pub mod header;
pub mod input;
pub mod layout;
pub mod live;
pub mod pane;
pub mod relive;
pub mod save;
pub mod screen;
pub mod status;
pub mod wrap;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hikasa_core::{CancelToken, JobKind, Pair, Phase, Side};
use hikasa_session::api::{
    LiveOptions, LiveTextView, TextCompareRequest, TextCompareResponse, ViewOutcome,
};
use hikasa_session::{ApiError, Service};
use hikasa_text::edit::{EditBlock, EditableDocument};
use hikasa_text::{
    CharClass, CompareObserver, Direction, ElementId, FilteredView, Importance, PartialRows,
    Recompared, RowIdx, RowState, TextComparison, TextDisplayFilter, TextDocument, TextToggles,
    ViewItem, ViewOptions,
};
use parking_lot::Mutex;
use serde::Serialize;

use crate::commands::{CommandId, Ctx};
use crate::job::Job;
use crate::status::{JobStatus, LogNote, Message, StatusItems};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;

pub use self::pane::PaneState;

/// Default font size of the panes (px).
pub const DEFAULT_FONT_PX: u32 = 13;
/// Smallest and largest font sizes.
const FONT_RANGE: (u32, u32) = (8, 32);
/// Longest displayed line of a slice (UTF-16 units); longer lines are cut with a marker (the panes lay out long
/// lines through [`wrap::LineGeom`]'s window instead).
pub const MAX_DISPLAY_COLUMNS: usize = 4096;
/// Largest slice one request may ask for.
pub const MAX_SLICE: u32 = 512;
/// Syntax colour slots (the palette has this many element colours after the default colour 0).
pub const PALETTE_SLOTS: u32 = 8;
/// Edits queued while a save of their document runs (D-300); more are refused.
pub const MAX_QUEUED: usize = 1024;

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

/// File information of one pane (the title line of the File Info header).
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

/// What the view asks of its window (taken by the app after each call: prompts, clipboard writes, notices).
#[derive(Debug, Clone, PartialEq)]
pub enum TextRequest {
    /// *"<name> contains binary data. Edit anyway?"* for this pane (D-297).
    AskBinary(Side),
    /// A line of the message panel (a blocked edit, *The clipboard holds no text.*, a refused queue).
    Message(Message),
    /// A warning notice with a log entry (a failed recompare, a failed relive).
    Warning(String),
    /// Places the primary selection (D-298).
    WritePrimary(String),
    /// A middle click: paste the primary selection at this pane's caret.
    PastePrimary(Side),
    /// A save asks a question (`save::asking`): the window shows its prompt.
    SaveAsked,
    /// Read this side again (a header path Accept, *Reopen with Encoding*): the window asks D-301 first.
    Reopen(Side),
    /// A relive (a format pick, D-304).
    Relive(relive::Cause),
    /// A save ended: *Saved <name>* (info) or a warning with *Show log*.
    Saved {
        /// The notice's text.
        text: String,
        /// Every item was written.
        ok: bool,
    },
}

/// An edit that waits for an answer or for a save (D-297's binary question, D-300's frozen documents).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEdit {
    /// The pane.
    pub side: Side,
    /// The request.
    pub request: hikasa_text::EditRequest,
    /// How it joins the undo log.
    pub origin: edit::Origin,
    /// The selection after it (`None`: the outcome's).
    pub selection: Option<hikasa_text::Selection>,
}

/// The Text Compare view model.
pub struct TextViewModel {
    request: TextCompareRequest,
    live_options: LiveOptions,
    titles: [Option<String>; 2],
    names: [String; 2],
    probe: Arc<TextProbe>,
    wake: Arc<dyn Fn() + Send + Sync>,
    job: Option<Job<LiveTextView>>,
    preview: Option<(Arc<TextDocument>, Arc<TextDocument>)>,
    view: Option<LiveTextView>,
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
    /// Disable editing (FC-003): the session's read-only sides and `--nobackups`.
    write: crate::folder_view::WritePolicy,
    // ---- M2 W11a: the editor -----------------------------------------------------------------------------------
    /// The two panes' carets and selections.
    panes: [PaneState; 2],
    /// The pane with the keyboard.
    focus: Side,
    /// Recompare scheduling (D-294).
    recompare: live::Recompare,
    rejob: Option<(u64, Job<Recompared>)>,
    /// The window's frame time (s): the debounce, the blink, the primary selection's settle.
    now: f64,
    /// Display flags of the view (WordWrap, Overwriting, ShowSpecInfo).
    layout: layout::Display,
    /// Installs of a recompare (and opens and relives): the thumbnail, statistics and Info refresh then.
    installs: u64,
    /// Thumbnails computed (a counter for the tests: never per keystroke).
    thumb_builds: AtomicU64,
    /// The binary question per pane: `None` not asked, `Some(false)` answered No (remembered until a reload).
    binary_answer: [Option<bool>; 2],
    /// The edit the binary question waits for.
    pending_binary: Option<PendingEdit>,
    /// Requests for the window.
    requests: Vec<TextRequest>,
    /// The save running on this view's documents (D-300): their edits wait in `queued`.
    saving: [bool; 2],
    queued: Vec<input::Queued>,
    /// A relive (a rule or format change keeping the edits, D-304).
    relive: Option<relive::Running>,
    /// The status bar, recomputed only when its inputs change (D-303).
    status_bar: status::Cache,
    /// The header's state per pane (path edits, open menus).
    pub(crate) headers: [header::HeaderState; 2],
    /// The primary selection's settle state (D-298).
    primary: clipboard::PrimaryState,
    /// The caret must come into view (the panes scroll at the next frame).
    pub(crate) reveal_caret: bool,
    /// Rows the panes scroll by at the next frame (Ctrl+Up/Down), taken by the window.
    pub(crate) scroll_rows: f64,
    /// The text width of each pane (display units; word wrap).
    text_width: [f32; 2],
    /// The save running on this view (D-300).
    save: Option<save::SaveFlow>,
    /// The request of a reopen the window was asked for.
    reopen: Option<TextCompareRequest>,
    /// The line a gutter press started at (a gutter drag selects from it).
    gutter_from: Option<u32>,
    /// The enabled text formats' names (the header's picker), set by the window.
    format_names: Vec<String>,
    /// How the last save ended (taken by the window: a close waiting for it).
    last_save: Option<save::Ended>,
}

/// Preparation of the syntax spans (`TextComparison::prepare_spans` on a worker when the live comparison's start
/// states are not prepared, §1.10; W11a keeps them through edits, so this runs after an open or a large change only).
enum SpanState {
    /// Nothing runs.
    Idle,
    /// The worker runs on this generation.
    Preparing(Job<()>),
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
        TextViewModel::with_live(service, request, view, titles, wake, LiveOptions::default())
    }

    /// [`TextViewModel::new`] with the view's live options (`--ro*`, `--savetarget`, `--nobackups`; D-314).
    pub fn with_live(
        service: &Service,
        request: TextCompareRequest,
        view: ViewOptions,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
        live_options: LiveOptions,
    ) -> TextViewModel {
        let names = [display_name(&request.left), display_name(&request.right)];
        let mut model = TextViewModel {
            request,
            live_options,
            titles,
            names,
            probe: Arc::new(TextProbe::default()),
            wake,
            job: None,
            preview: None,
            view: None,
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
            write: crate::folder_view::WritePolicy::default(),
            panes: [PaneState::new(Side::Left), PaneState::new(Side::Right)],
            focus: Side::Left,
            recompare: live::Recompare::default(),
            rejob: None,
            now: 0.0,
            layout: layout::Display::default(),
            installs: 0,
            thumb_builds: AtomicU64::new(0),
            binary_answer: [None, None],
            pending_binary: None,
            requests: Vec::new(),
            saving: [false, false],
            queued: Vec::new(),
            relive: None,
            status_bar: status::Cache::default(),
            headers: [
                header::HeaderState::default(),
                header::HeaderState::default(),
            ],
            primary: clipboard::PrimaryState::default(),
            reveal_caret: false,
            scroll_rows: 0.0,
            text_width: [f32::MAX, f32::MAX],
            save: None,
            reopen: None,
            gutter_from: None,
            format_names: Vec::new(),
            last_save: None,
        };
        model.start(service);
        model
    }

    /// Sets what the view may write (FC-003): the read-only sides become `SessionReadOnly` blocks of the panes.
    pub fn set_write_policy(&mut self, write: crate::folder_view::WritePolicy) {
        self.write = write;
        self.apply_write_policy();
    }

    /// What the view may write.
    pub fn write_policy(&self) -> crate::folder_view::WritePolicy {
        self.write
    }

    /// The panes' blocks of the write policy: a read-only side is `SessionReadOnly` unless the command line already
    /// blocks it (`CommandLine`).
    fn apply_write_policy(&mut self) {
        let ro = self.write.read_only;
        if let Some(view) = self.view.as_mut() {
            for (side, doc) in [
                (Side::Left, &mut view.docs.left),
                (Side::Right, &mut view.docs.right),
            ] {
                let mut e = doc.editability();
                let wanted = ro.get(side) && !e.blocks.contains(&EditBlock::CommandLine);
                let has = e.blocks.contains(&EditBlock::SessionReadOnly);
                if wanted && !has {
                    e.add(EditBlock::SessionReadOnly);
                    doc.set_editability(e);
                } else if !wanted && has && !self.live_options.session_read_only.get(side) {
                    e.blocks.retain(|b| *b != EditBlock::SessionReadOnly);
                    doc.set_editability(e);
                }
            }
            self.bump();
        }
    }

    /// (Re)starts the comparison (Reload): the documents are read again, their edits dropped (the window asked first,
    /// D-301).
    fn start(&mut self, service: &Service) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
        if let SpanState::Preparing(job) = std::mem::replace(&mut self.spans, SpanState::Idle) {
            job.cancel();
        }
        if let Some((_, job)) = self.rejob.take() {
            job.cancel();
        }
        if let Some(running) = self.relive.take() {
            running.cancel();
        }
        self.recompare = live::Recompare::default();
        self.probe = Arc::new(TextProbe::default());
        self.preview = None;
        self.view = None;
        self.error = None;
        self.cancelled = false;
        self.filtered = None;
        self.cursor = None;
        self.binary_answer = [None, None];
        self.pending_binary = None;
        self.queued.clear();
        self.started = Instant::now();
        self.loaded_after = None;
        self.ready_after = None;
        self.bump();
        let request = self.request.clone();
        let live = self.live_options.clone();
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
            move |svc, job| svc.text_compare_live(&request, live, job, Some(&mut observer)),
        ));
    }

    /// The comparison request.
    pub fn request(&self) -> &TextCompareRequest {
        &self.request
    }

    /// The header's path Accept or *Reopen with Encoding* of `side` (D-302): `spec` the new spec (`None`: the same),
    /// `encoding` the new override (`None`: unchanged; `Some(None)`: detect). The window asks D-301 first.
    pub fn request_reopen(
        &mut self,
        side: Side,
        spec: Option<std::ffi::OsString>,
        encoding: Option<Option<String>>,
    ) {
        let mut req = self.reopen.take().unwrap_or_else(|| self.request.clone());
        let k = index_of(side);
        if let Some(spec) = spec {
            if k == 1 {
                req.right = spec;
            } else {
                req.left = spec;
            }
        }
        if let Some(enc) = encoding {
            if k == 1 {
                req.encodings.right = enc;
            } else {
                req.encodings.left = enc;
            }
        }
        self.reopen = Some(req);
        self.requests.push(TextRequest::Reopen(side));
    }

    /// The enabled text formats' names the header offers (FF-003).
    pub fn set_format_names(&mut self, names: Vec<String>) {
        self.format_names = names;
    }

    /// The names the header's format picker offers.
    pub fn format_names(&self) -> &[String] {
        &self.format_names
    }

    /// The request of the reopen asked last (the view's own when none).
    pub fn take_reopen_request(&mut self) -> TextCompareRequest {
        self.reopen.take().unwrap_or_else(|| self.request.clone())
    }

    /// The header's format pick of `side` (`None`: detect): a relive keeping the edits (D-304).
    pub fn request_format(&mut self, side: Side, format: Option<String>) {
        self.requests
            .push(TextRequest::Relive(relive::Cause::Format(side, format)));
    }

    /// True when edits of `side` wait (a save of that pane or a relive runs, D-300, D-304).
    pub fn frozen(&self, side: Side) -> bool {
        self.saving[index_of(side)] || self.relive.is_some()
    }

    /// *Ignore Unimportant Differences* of the view.
    pub fn ignore_unimportant(&self) -> bool {
        self.opts.ignore_unimportant
    }

    /// `Loaded in 0.27 s · compared in 0.61 s` (the last full compare or relive), when known.
    pub fn timings_text(&self) -> Option<String> {
        let loaded = self.loaded_after?;
        let ready = self.ready_after?;
        Some(format!(
            "Loaded in {:.2} s · compared in {:.2} s",
            loaded.as_secs_f64(),
            ready.saturating_sub(loaded).as_secs_f64()
        ))
    }

    /// What keeps the view busy (the large-pair probe).
    #[cfg(test)]
    pub fn busy_parts(&self) -> String {
        format!(
            "job {} spans {} rejob {} relive {} save {} recompare {:?} installs {} provisional {:?}",
            self.job.is_some(),
            matches!(self.spans, SpanState::Preparing(_)),
            self.rejob.is_some(),
            self.relive.is_some(),
            save::working(self),
            self.recompare,
            self.installs,
            self.comparison().map(|c| c.provisional()),
        )
    }

    /// Status bar builds so far (tests: per caret move, never per frame).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn status_builds(&self) -> u64 {
        self.status_bar.builds.get()
    }

    /// Tab caption: the files' names, `*` after a modified one.
    pub fn caption(&self) -> String {
        let star = |k: usize| {
            if self
                .doc(side_of(k))
                .is_some_and(EditableDocument::is_modified)
            {
                "*"
            } else {
                ""
            }
        };
        let names = [
            format!("{}{}", self.names[0], star(0)),
            format!("{}{}", self.names[1], star(1)),
        ];
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&names[0], &names[1]],
        )
    }

    /// The probe of the running comparison (tests: it lives as long as the job's observer).
    #[cfg(test)]
    pub fn probe(&self) -> Arc<TextProbe> {
        Arc::clone(&self.probe)
    }

    /// The current view options (filter, ignore-unimportant, expanded gaps).
    pub fn view_options(&self) -> &ViewOptions {
        &self.opts
    }

    /// The response of the open (or of the last relive): the formats and their reasons, the warnings. Its
    /// `comparison` is the one of that moment; [`TextViewModel::comparison`] is the live one.
    pub fn response(&self) -> Option<&TextCompareResponse> {
        self.view.as_ref().map(|v| &v.response)
    }

    /// The live comparison's current rows (valid for the documents at every moment).
    pub fn comparison(&self) -> Option<&Arc<TextComparison>> {
        self.view.as_ref().map(|v| v.live.current())
    }

    /// The live view (comparison, documents, specs).
    pub fn live_view(&self) -> Option<&LiveTextView> {
        self.view.as_ref()
    }

    /// The document of a pane.
    pub fn doc(&self, side: Side) -> Option<&EditableDocument> {
        self.view.as_ref().map(|v| match side {
            Side::Right => &v.docs.right,
            _ => &v.docs.left,
        })
    }

    /// Both documents.
    pub fn docs(&self) -> Option<Pair<&EditableDocument>> {
        self.view.as_ref().map(|v| v.docs.as_ref())
    }

    /// A pane's caret and selection.
    pub fn pane(&self, side: Side) -> &PaneState {
        &self.panes[usize::from(side == Side::Right)]
    }

    /// A pane's caret and selection, to change.
    pub fn pane_mut(&mut self, side: Side) -> &mut PaneState {
        &mut self.panes[usize::from(side == Side::Right)]
    }

    /// The pane with the keyboard.
    pub fn focus(&self) -> Side {
        self.focus
    }

    /// Gives the keyboard to a pane (a click, `GoLeft`/`GoRight`).
    pub fn set_focus(&mut self, side: Side) {
        if self.focus != side {
            self.focus = side;
            self.reveal_caret = true;
            self.dirty = true;
        }
    }

    /// The display flags (word wrap, overwrite, File Info header).
    pub fn display(&self) -> &layout::Display {
        &self.layout
    }

    /// Time from the view's start to the loaded documents and to the finished comparison.
    pub fn timings(&self) -> (Option<Duration>, Option<Duration>) {
        (self.loaded_after, self.ready_after)
    }

    /// Recompares, relives and opens installed so far (the thumbnail and statistics refresh then).
    pub fn installs(&self) -> u64 {
        self.installs
    }

    /// Thumbnails computed so far.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn thumb_builds(&self) -> u64 {
        self.thumb_builds.load(Ordering::Relaxed)
    }

    /// The recompare scheduling.
    pub fn recompare_state(&self) -> &live::Recompare {
        &self.recompare
    }

    /// The window's frame time (s), set each frame before the view is polled and drawn.
    pub fn set_now(&mut self, now: f64) {
        self.now = now;
    }

    /// The frame time the view last saw.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// True while a job of the view runs.
    pub fn busy(&self) -> bool {
        self.job.is_some()
            || matches!(self.spans, SpanState::Preparing(_))
            || self.rejob.is_some()
            || self.relive.is_some()
            || save::working(self)
    }

    /// True while a job of the view has ended but its result is not published yet (the window looks again at
    /// once).
    pub fn settling(&self) -> bool {
        self.job.as_ref().is_some_and(Job::settling)
            || matches!(&self.spans, SpanState::Preparing(job) if job.settling())
            || self.rejob.as_ref().is_some_and(|(_, j)| j.settling())
            || self.relive.as_ref().is_some_and(relive::Running::settling)
    }

    /// When the view wants a frame without input (a recompare's debounce, the primary selection's settle, the
    /// status's *Recomparing…*), in the frame clock.
    pub fn wake_at(&self) -> Option<f64> {
        let mut at = self.recompare.due_at();
        if let Some(t) = self.primary.due_at() {
            at = Some(at.map_or(t, |a| a.min(t)));
        }
        if let Some(s) = self.recompare.started_at {
            let t = s + live::SHOW_AFTER_S;
            if t > self.now {
                at = Some(at.map_or(t, |a| a.min(t)));
            }
        }
        at
    }

    /// True when the syntax spans can be drawn.
    pub fn spans_ready(&self) -> bool {
        self.comparison()
            .is_some_and(|c| c.spans_prepared(Side::Left) && c.spans_prepared(Side::Right))
    }

    /// Stops every job (the view is closing).
    pub fn cancel(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
        if let SpanState::Preparing(job) = &self.spans {
            job.cancel();
        }
        if let Some((_, job)) = &self.rejob {
            job.cancel();
        }
        if let Some(running) = &self.relive {
            running.cancel();
        }
        save::cancel(self);
    }

    /// True once since the last call when the panes must refetch their meta.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// The requests for the window since the last call.
    pub fn take_requests(&mut self) -> Vec<TextRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Asks the window for something.
    pub(crate) fn request_window(&mut self, request: TextRequest) {
        self.requests.push(request);
    }

    /// A new data generation: the panes rebuild their slice.
    fn bump(&mut self) {
        self.generation += 1;
        self.dirty = true;
    }

    /// The data generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Polls the jobs (once per frame); true when something changed. A recompare that is due starts here.
    pub fn poll(&mut self, service: &Service) -> bool {
        let mut changed = false;
        let arrived = if self.preview.is_none() && self.view.is_none() {
            self.probe.docs.lock().clone()
        } else {
            None
        };
        if let Some(docs) = arrived {
            self.preview = Some(docs);
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
                Ok(view) => {
                    self.opened(view);
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
        changed |= self.poll_recompare();
        changed |= relive::poll(self, service);
        changed |= save::poll(self, service);
        if let SpanState::Preparing(job) = &self.spans
            && let Some(result) = job.try_result()
        {
            let _ = result;
            self.spans = SpanState::Idle;
            self.bump();
            changed = true;
        }
        changed |= self.start_due(service);
        self.ensure_spans(service);
        changed
    }

    /// The live comparison arrived (an open or a reload).
    fn opened(&mut self, view: LiveTextView) {
        self.ready_after = Some(self.started.elapsed());
        self.preview = None;
        self.view = Some(view);
        self.apply_write_policy();
        for k in 0..2 {
            if let Some(doc) = self.doc(side_of(k)).cloned() {
                self.panes[k].clamp(&doc);
            }
        }
        self.installed();
        self.rebuild();
    }

    /// After an open, an install or a relive: the thumbnail, the statistics and the Info figures change now.
    fn installed(&mut self) {
        self.installs += 1;
        self.bump();
    }

    /// Starts preparing the syntax spans on a worker when the live comparison has none (a multi-line grammar).
    fn ensure_spans(&mut self, service: &Service) {
        if !matches!(self.spans, SpanState::Idle) || self.spans_ready() {
            return;
        }
        let Some(cmp) = self.comparison().cloned() else {
            return;
        };
        // Not while edits wait for a recompare: the comparison changes at every key, the next install keeps them.
        if !self.recompare.idle() {
            return;
        }
        self.spans = SpanState::Preparing(Job::spawn(
            service,
            JobKind::TextCompare,
            "syntax spans".into(),
            &self.wake,
            move |_svc, job| {
                let cancel: &CancelToken = &job.cancel;
                cmp.prepare_spans(Side::Left, cancel)
                    .and_then(|()| cmp.prepare_spans(Side::Right, cancel))
                    .map_err(|_| ApiError::Cancelled)
            },
        ));
    }

    /// Starts the recompare when it is due (D-294): one job at a time, on snapshots.
    fn start_due(&mut self, service: &Service) -> bool {
        if self.relive.is_some() || !self.recompare.due(self.now) {
            return false;
        }
        let Some(view) = self.view.as_ref() else {
            return false;
        };
        let job = view.live.recompare_job(view.docs.as_ref());
        let generation = job.generation();
        let handle = Job::spawn(
            service,
            JobKind::TextRecompare,
            "recompare".into(),
            &self.wake,
            move |svc, control| {
                job.run(Some(svc.pool()), &control.ctl()).map_err(|e| {
                    if e.is_cancelled() {
                        ApiError::Cancelled
                    } else {
                        ApiError::Text(e)
                    }
                })
            },
        );
        self.recompare.start(handle.id(), generation, self.now);
        self.rejob = Some((generation, handle));
        true
    }

    /// A recompare landed: installed (`Replayed` schedules the next at once); a failure is a warning notice and the
    /// view stays provisional until the next edit.
    fn poll_recompare(&mut self) -> bool {
        let Some((_, job)) = &self.rejob else {
            return false;
        };
        let Some(result) = job.try_result() else {
            return false;
        };
        self.rejob = None;
        match result {
            Ok(done) => {
                let Some(view) = self.view.as_mut() else {
                    return true;
                };
                let installed = view.live.install(done, view.docs.as_ref());
                for warning in view.live.take_warnings() {
                    self.notes.push(LogNote {
                        kind: LogKind::Warning,
                        text: warning.message,
                    });
                }
                self.recompare.landed(self.now, installed);
                self.installed();
                self.rebuild_keep();
            }
            Err(ApiError::Cancelled) => self.recompare.failed(),
            Err(e) => {
                self.recompare.failed();
                self.requests.push(TextRequest::Warning(format!(
                    "The comparison could not be updated: {e}"
                )));
            }
        }
        true
    }

    /// Rebuilds the filtered view after a filter, ignore-unimportant or gap change (the cursor row is brought into
    /// view).
    fn rebuild(&mut self) {
        self.filtered = self.comparison().map(|c| c.view(&self.opts));
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

    /// Rebuilds the filtered view after an edit or an install, keeping the scroll position.
    pub(crate) fn rebuild_keep(&mut self) {
        self.filtered = self.comparison().map(|c| c.view(&self.opts));
        self.bump();
    }

    /// The phase.
    pub fn phase(&self) -> TextPhase {
        if self.view.is_some() {
            TextPhase::Ready
        } else if self.error.is_some() || self.cancelled {
            TextPhase::Failed
        } else if self.preview.is_some() {
            TextPhase::Loaded
        } else {
            TextPhase::Loading
        }
    }

    /// Number of display positions.
    pub fn len(&self) -> u32 {
        match (&self.filtered, &self.preview) {
            (Some(view), _) => view.len(),
            (None, Some((l, r))) => l.line_count().max(r.line_count()),
            _ => 0,
        }
    }

    /// The filtered view (display positions → rows).
    pub fn filtered(&self) -> Option<&FilteredView> {
        self.filtered.as_ref()
    }

    /// The preview documents (loaded, not aligned yet).
    pub(crate) fn preview(&self) -> Option<&(Arc<TextDocument>, Arc<TextDocument>)> {
        self.preview.as_ref()
    }

    /// The panes' meta (consumes the one-shot scroll request).
    pub fn meta(&mut self) -> TextMeta {
        let rows = self.comparison().map_or(0, |c| c.row_count());
        let lines = match (self.docs(), &self.preview) {
            (Some(d), _) => d.left.line_count().max(d.right.line_count()),
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
            _ if self.len() == 0 && lines > 0 => {
                "No lines are shown by the display filter.".to_owned()
            }
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
            syntax: self.view.is_some() && self.spans_ready(),
            digits: digits(lines.saturating_add(1)),
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

    /// The cursor row (the current row of navigation: the focus pane's caret row).
    pub fn cursor(&self) -> Option<RowIdx> {
        self.cursor
    }

    /// Display positions `start..start + count` (clamped to [`MAX_SLICE`]).
    pub fn slice(&self, start: u32, count: u32) -> TextSlice {
        let end = start.saturating_add(count.min(MAX_SLICE)).min(self.len());
        let mut rows = Vec::with_capacity(end.saturating_sub(start) as usize);
        match (&self.view, &self.filtered, &self.preview) {
            (Some(view), Some(filtered), _) => {
                let cmp = view.live.current();
                let syntax = self.spans_ready();
                for pos in start..end {
                    match filtered.get(pos) {
                        Some(ViewItem::Row { row }) => {
                            rows.push(self.aligned_row(view, cmp, row, syntax));
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
        view: &LiveTextView,
        cmp: &TextComparison,
        row: RowIdx,
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
        // A settled row's inline differences classify its tokens: without prepared start states (after an edit that
        // dropped them, until the worker prepares them again) that would tokenize the document on the UI thread, so
        // they wait for the states; a pending row's come from its live lines (cheap).
        let inline = match r.state {
            RowState::Changed | RowState::Same if syntax || view.live.is_pending(row) => {
                view.live.inline(row, view.docs.as_ref())
            }
            _ => None,
        };
        let mut out = SliceRow {
            k: 0,
            st: state_code(r.state),
            im: importance_code(r.importance),
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
            let doc = if k == 1 {
                &view.docs.right
            } else {
                &view.docs.left
            };
            let tab = doc.options().tab_size.max(1);
            let text = doc.line(line);
            let layout = layout_line(&text, usize::from(tab));
            out.ln[k] = doc.document().physical_line(line);
            out.sp[k] = if syntax {
                segments(
                    &layout,
                    cmp.spans(side, line)
                        .iter()
                        .map(|s| (s.range.start, s.range.end, span_colour(s.class, s.element))),
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

    /// The thumbnail with `height` buckets (TC-032), of the last install (never per keystroke: the window caches it
    /// by [`TextViewModel::installs`]).
    pub fn thumbnail(&self, height: u32) -> ThumbData {
        self.thumb_builds.fetch_add(1, Ordering::Relaxed);
        let cells = self.comparison().map_or_else(Vec::new, |c| {
            c.thumbnail(height.clamp(1, 8192), self.opts.ignore_unimportant)
                .into_iter()
                .map(|c| c.bits())
                .collect()
        });
        ThumbData {
            generation: self.installs,
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
        let cmp = self.comparison()?;
        let iu = self.opts.ignore_unimportant;
        let id = match self.cursor {
            Some(cursor) => cmp.next_section(cursor, dir, iu)?,
            None => cmp.next_section_from(RowIdx(self.top_row()?), dir, iu)?,
        };
        cmp.sections().get(id.0 as usize).map(|s| s.rows.start)
    }

    /// Moves the cursor (and the focus pane's caret) to the first row of the next/previous section.
    fn go_to_section(&mut self, dir: Direction) -> bool {
        let Some(row) = self.section_target(dir) else {
            return false;
        };
        self.go_to_row(row);
        true
    }

    /// Makes `row` the cursor row: the focus pane's caret goes to its line (or the next line of that side).
    pub(crate) fn go_to_row(&mut self, row: u32) {
        self.cursor = Some(RowIdx(row));
        self.scroll = Some(self.position_of_row(row));
        let side = self.focus;
        if let Some(line) = self.line_at_row(side, row) {
            let pane = self.pane_mut(side);
            pane.set_caret(hikasa_text::TextPos::new(line, 0), false);
            pane.sticky_x = None;
        }
        self.dirty = true;
    }

    /// The line of `side` a row shows, else the next line of that side (a gap row of that pane), else its end.
    pub fn line_at_row(&self, side: Side, row: u32) -> Option<u32> {
        let cmp = self.comparison()?;
        let line_of = |r: hikasa_text::Row| match side {
            Side::Right => r.right,
            _ => r.left,
        };
        if let Some(line) = cmp.row(RowIdx(row)).and_then(line_of) {
            return Some(line.0);
        }
        let next = hikasa_text::nav::existing_line(cmp, RowIdx(row), side, Direction::Forward)
            .and_then(|r| cmp.row(r))
            .and_then(line_of);
        Some(next.map_or_else(
            || {
                self.doc(side)
                    .map_or(0, |d| d.position_lines().saturating_sub(1))
            },
            |l| l.0,
        ))
    }

    /// The row showing `line` of `side` (the caret's row).
    pub fn row_of_line(&self, side: Side, line: u32) -> Option<u32> {
        let cmp = self.comparison()?;
        cmp.row_for_line(side, hikasa_text::LineIdx(line))
            .map(|r| r.0)
            .or_else(|| {
                // The empty line after a terminated last line: the last row.
                (line >= self.doc(side)?.line_count()).then(|| cmp.row_count().saturating_sub(1))
            })
    }

    /// The caret moved: the cursor row follows the focus pane's caret, the panes bring it into view.
    pub(crate) fn caret_moved(&mut self) {
        let side = self.focus;
        let line = self.pane(side).caret.line.0;
        if let Some(row) = self.row_of_line(side, line) {
            self.cursor = Some(RowIdx(row));
        }
        self.reveal_caret = true;
        self.dirty = true;
        clipboard::selection_changed(self);
    }

    /// Fills the command context of this view.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.busy = self.job.is_some();
        ctx.ready = self.view.is_some();
        ctx.next_difference = self.section_target(Direction::Forward).is_some();
        ctx.prev_difference = self.section_target(Direction::Backward).is_some();
        // Compare Parent Folders: always (FC-004).
        ctx.parents_available = true;
        // M2 W11a (§1.11.28): the editor's predicates.
        let focus = self.focus;
        ctx.focus_side = Some(focus);
        ctx.two_panes = true;
        let editable = |side: Side| {
            self.doc(side)
                .is_some_and(|d| d.editability().editable() || only_binary(d))
        };
        ctx.pane_editable = [editable(Side::Left), editable(Side::Right)];
        ctx.has_selection = self.pane(focus).has_selection();
        ctx.can_undo = self.doc(focus).is_some_and(EditableDocument::can_undo);
        ctx.can_redo = self.doc(focus).is_some_and(EditableDocument::can_redo);
        ctx.modified = self.doc(focus).is_some_and(EditableDocument::is_modified);
        ctx.any_modified = [Side::Left, Side::Right]
            .into_iter()
            .any(|s| self.doc(s).is_some_and(EditableDocument::is_modified));
        ctx.saving = self.saving[0] || self.saving[1];
        ctx.installed = self.comparison().is_some_and(|c| !c.provisional());
        ctx.has_anchors = self.comparison().is_some_and(|c| !c.anchors().is_empty());
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
            CommandId::WordWrap => self.layout.word_wrap,
            CommandId::Overwriting => self.layout.overwrite,
            CommandId::ShowSpecInfo => self.layout.header,
            _ => return None,
        })
    }

    /// Executes a command of this view; false when the id is not one of its commands (the window's clipboard and
    /// save commands are `app/text.rs`'s).
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
            CommandId::WordWrap => {
                self.layout.word_wrap = !self.layout.word_wrap;
                self.reveal_caret = true;
                self.bump();
            }
            CommandId::Overwriting => {
                self.layout.overwrite = !self.layout.overwrite;
                self.dirty = true;
            }
            CommandId::ShowSpecInfo => {
                self.layout.header = !self.layout.header;
                self.dirty = true;
            }
            id => return edit::command(self, id),
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

    /// Applies the layout of the view type (D-108): `font_size` (else the theme's editor size `default_px`),
    /// `thumbnail`, `word_wrap` and `header`. A value of the wrong type is ignored.
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
        self.layout.read_layout(layout, view);
    }

    /// The layout keys this view owns (D-108): `font_size`, `thumbnail`, `word_wrap`, `header`.
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        let mut keys = vec![
            ("font_size", LayoutValue::Float(f64::from(self.font_px))),
            ("thumbnail", LayoutValue::Bool(self.show_thumbnail)),
        ];
        keys.extend(self.layout.keys());
        keys
    }

    /// The inputs watched for changes on disk (D-118): the two specs as given.
    pub fn inputs(&self) -> [String; 2] {
        [lossy(&self.request.left), lossy(&self.request.right)]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, D-107): the files as they are on disk.
    pub fn duplicate(&self) -> (hikasa_session::api::ViewRequest, [Option<String>; 2]) {
        (
            hikasa_session::api::ViewRequest::Text {
                request: self.request.clone(),
                view: self.opts.clone(),
            },
            self.titles.clone(),
        )
    }

    /// The running jobs (the job indicator, D-115): a recompare only after [`live::SHOW_AFTER_S`].
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
        if let Some((_, job)) = &self.rejob
            && self.recompare.showing(self.now)
        {
            out.push(JobStatus {
                view,
                title: job.title().to_owned(),
                phase: None,
                fraction: None,
            });
        }
        out.extend(relive::jobs(self, view));
        out.extend(save::jobs(self, view));
        out
    }

    /// The notes for the log since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    /// Adds a note for the log.
    pub(crate) fn note(&mut self, kind: LogKind, text: String) {
        self.notes.push(LogNote { kind, text });
    }

    /// The summary of a finished comparison for the log: the verdict, the sections and the duration.
    fn summary_note(&self) -> Option<LogNote> {
        let cmp = self.comparison()?;
        let stats = cmp.stats();
        let verdict = cmp.verdict(self.opts.ignore_unimportant).kind();
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

    /// The status bar (D-115, D-303): the comparison state on the left; the caret, the mode, the row, the element,
    /// the encoding, the line ending, the editability and the times on the right.
    pub fn status_items(&self) -> StatusItems {
        status::items(self)
    }

    fn set_font(&mut self, px: u32) {
        self.font_px = px.clamp(FONT_RANGE.0, FONT_RANGE.1);
        self.dirty = true;
    }

    /// The D-23 outcome when the view closes (§1.8.7 item 22): the verdict under the view's current
    /// ignore-unimportant (of the last install, §1.11.3), `LoadFailed` when the inputs could not be compared,
    /// `NoVerdict` while comparing or after a cancellation.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        // A result that arrived since the last frame counts.
        self.poll(service);
        if let Some(cmp) = self.comparison() {
            let iu = self.opts.ignore_unimportant;
            ViewOutcome::Text {
                result: cmp.verdict(iu),
                ignore_unimportant: iu,
            }
        } else if self.error.is_some() {
            ViewOutcome::LoadFailed
        } else {
            ViewOutcome::NoVerdict
        }
    }

    /// Waits up to `limit` for a running recompare to land (a view closed with a provisional comparison and edits,
    /// §1.11.3); true when the comparison is exact afterwards.
    pub fn settle_recompare(&mut self, service: &Service, limit: Duration) -> bool {
        let began = Instant::now();
        loop {
            // Edits still in the debounce start their recompare now.
            self.now += live::DEBOUNCE_S;
            self.poll(service);
            if self.rejob.is_none() && self.recompare.idle() {
                return self.comparison().is_some_and(|c| !c.provisional());
            }
            if began.elapsed() >= limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Pane headers and status bar texts.
    pub fn status(&self) -> TextStatus {
        let mut status = TextStatus {
            panes: [Side::Left, Side::Right].map(|side| PaneInfo {
                title: crate::shared::shown_path(&header::title(self, side)),
                detail: String::new(),
            }),
            error: self.error.as_deref().map(crate::shared::shown_path),
            ..TextStatus::default()
        };
        for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            status.panes[k].detail = header::detail(self, side);
        }
        status.state = match (self.comparison(), self.phase()) {
            (Some(cmp), _) => {
                let stats = cmp.stats();
                let iu = self.opts.ignore_unimportant;
                let verdict = cmp.verdict(iu).kind();
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
        if let (Some(r), Some(cmp)) = (self.response(), self.comparison()) {
            let syntax = if self.spans_ready() {
                ""
            } else {
                " · preparing syntax colours"
            };
            status.detail = format!(
                "{} · {:?} · loaded in {} ms, compared in {} ms{syntax}",
                r.format.name,
                cmp.algorithm(),
                self.loaded_after.map_or(0, |d| d.as_millis()),
                self.ready_after.map_or(0, |d| d.as_millis()),
            );
        }
        status
    }
}

/// The file name a notice names (`Saved a.txt`).
pub fn name_of(spec: &str) -> String {
    display_name(std::ffi::OsStr::new(spec))
}

/// The side of index `k` (0 left, 1 right).
pub fn side_of(k: usize) -> Side {
    if k == 1 { Side::Right } else { Side::Left }
}

/// The index of a side (0 left, 1 right).
pub fn index_of(side: Side) -> usize {
    usize::from(side == Side::Right)
}

/// True when the only blocks of a document are `Binary` (the user may lift it, D-297).
pub fn only_binary(doc: &EditableDocument) -> bool {
    let e = doc.editability();
    !e.blocks.is_empty() && e.blocks.iter().all(|b| *b == EditBlock::Binary)
}

/// The slice code of a row state.
pub fn state_code(state: RowState) -> u8 {
    match state {
        RowState::Same => 0,
        RowState::Changed => 1,
        RowState::LeftOnly => 2,
        RowState::RightOnly => 3,
    }
}

/// The slice code of an importance.
pub fn importance_code(importance: Importance) -> u8 {
    match importance {
        Importance::None => 0,
        Importance::Unimportant => 1,
        Importance::Important => 2,
    }
}

/// The colour slot of a span: an element's slot, 0 for everything else.
pub fn span_colour(class: CharClass, element: ElementId) -> u32 {
    match class {
        CharClass::Element if element != ElementId::EVERYTHING_ELSE => colour_slot(element),
        _ => 0,
    }
}

/// Encoding, line endings and line count of a document.
pub(crate) fn doc_detail(doc: &TextDocument) -> String {
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
pub(crate) fn verdict_text(kind: hikasa_core::ContentKind) -> &'static str {
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
