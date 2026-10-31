//! The Folder Compare view model (docs/04 §4–§7; contract §1.10, §3.7): the comparison runs as jobs
//! (`folder_prepare`, then `folder_run` into the live model), the tree is rebuilt from the model whenever its
//! generation changes (incremental results while scanning, D-55), and the window of rows on screen is turned
//! into drawing data. States, colours, visibility, statistics and verdicts are `TreeRead` calls.

use std::cell::Cell as Memo;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hikasa_core::{CompareVerdict, JobKind, Side};
use hikasa_folder::{
    Category, ChannelObserver, ColourClass, DisplayOptions, EntryView, FolderDisplayFilter,
    FolderMode, FolderToggles, NodeId, TreeRead, VisibleRow,
};
use hikasa_session::api::{
    FolderCompareRequest, FolderCompareResponse, PreparedFolderCompare, TextCompareRequest,
    ViewOutcome,
};
use hikasa_session::{ApiError, Service};
use hikasa_text::{TextDisplayFilter, ViewOptions};

use crate::commands::{CommandId, Ctx};
use crate::job::Job;
use crate::status::{JobStatus, LogNote, StatusItems};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;

/// Default height of the log panel (points, `[layout.folder-compare] log_height`).
pub const LOG_HEIGHT: f64 = 160.0;

/// Height of one tree row (logical px).
pub const ROW_HEIGHT: u32 = 22;
/// Rows rendered above and below the visible ones.
const OVERSCAN: u32 = 12;
/// While scanning, the tree is rebuilt at most this often (the model changes thousands of times a second).
pub const REBUILD_INTERVAL: Duration = Duration::from_millis(66);
/// Capacity of the scan event channel (the drain thread empties it at once, so the scan never waits).
const EVENT_CAPACITY: usize = 1024;

/// One side's cell of a tree row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Name (control characters escaped).
    pub name: String,
    /// Size text (folders: the total below once scanned).
    pub size: String,
    /// Modification time (UTC).
    pub modified: String,
    /// Colour class of the colour spot and the name (FC-031; the palette maps it to a colour).
    pub colour: ColourClass,
}

/// One rendered tree row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    /// Index in the visible rows.
    pub index: u32,
    /// Node id.
    pub node: u32,
    /// Depth (indentation).
    pub depth: u32,
    /// A folder.
    pub folder: bool,
    /// A folder that is open.
    pub open: bool,
    /// A folder whose children are known.
    pub has_children: bool,
    /// Left cell.
    pub left: Option<Cell>,
    /// Right cell.
    pub right: Option<Cell>,
    /// Centre mark (state symbol).
    pub mark: &'static str,
    /// Centre mark tooltip.
    pub mark_title: &'static str,
    /// The row's category (the palette tells newer from different and older from unknown, FC-031).
    pub category: Category,
    /// Selected.
    pub selected: bool,
    /// Error details (tooltip).
    pub errors: String,
}

/// What the tree renders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderWindow {
    /// Visible rows in total (scroll height).
    pub total: u32,
    /// Rendered rows (a window around the scroll position).
    pub rows: Vec<RowView>,
}

/// Status bar and header texts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderStatus {
    /// Base folders.
    pub bases: [String; 2],
    /// Comparison state and statistics.
    pub state: String,
    /// Detail (timing, events).
    pub detail: String,
    /// Error.
    pub error: Option<String>,
}

enum FolderState {
    /// `folder_prepare` runs (it may read rules files on a slow share, §1.10).
    Preparing(Job<PreparedFolderCompare>),
    /// `folder_run` fills the live model.
    Running {
        prepared: PreparedFolderCompare,
        job: Job<FolderCompareResponse>,
    },
    /// The scan returned (complete, or incomplete after Stop).
    Done {
        prepared: PreparedFolderCompare,
        response: Box<FolderCompareResponse>,
    },
    /// Stop was pressed while preparing (no scan starts, §1.10) or the scan returned `Cancelled`.
    Stopped {
        prepared: Option<PreparedFolderCompare>,
    },
    /// The comparison could not start or the bases could not be loaded.
    Failed {
        prepared: Option<PreparedFolderCompare>,
        error: String,
    },
}

/// Timing of tree rebuilds (incremental rendering, acceptance (b)).
#[derive(Debug, Clone, Default)]
pub struct RebuildStats {
    /// Rebuild durations (ms), at most 100,000 samples.
    pub samples: Vec<f64>,
    /// Rows after the last rebuild.
    pub rows: usize,
    /// Rebuilds while the scan ran.
    pub during_scan: usize,
}

/// The Folder Compare view model.
pub struct FolderViewModel {
    request: FolderCompareRequest,
    titles: [Option<String>; 2],
    names: [String; 2],
    display: DisplayOptions,
    expand_all: bool,
    expanded: HashSet<NodeId>,
    collapsed: HashSet<NodeId>,
    state: FolderState,
    rows: Vec<VisibleRow>,
    seen_generation: u64,
    rebuilt_at: Option<Instant>,
    selected: Option<NodeId>,
    selected_index: Option<usize>,
    first: u32,
    viewport: u32,
    scroll_to: Option<u32>,
    events: Arc<AtomicU64>,
    wake: Arc<dyn Fn() + Send + Sync>,
    started: Instant,
    finished_after: Option<Duration>,
    stats: RebuildStats,
    /// Bumped by every rebuild of `rows`.
    rows_revision: u64,
    /// Next/previous-difference availability for (rows revision, selection, ignore-unimportant): computed when
    /// first asked after a change, not on every render (§1.10, O(visible) per frame).
    differences: Memo<Option<DifferenceMemo>>,
    /// How often the rows were walked for it.
    difference_walks: Memo<u64>,
    /// The log panel is shown (`ShowLog`, `[layout] log_panel`).
    show_log: bool,
    /// The log panel's height (points, `[layout] log_height`).
    log_height: f64,
    /// What the log learns when a job ends (D-116), taken by the app.
    notes: Vec<LogNote>,
}

/// The cached availability of next/previous difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DifferenceMemo {
    key: (u64, Option<usize>, bool),
    next: bool,
    prev: bool,
}

impl FolderViewModel {
    /// Creates the view and starts `folder_prepare` (then `folder_run`).
    pub fn new(
        service: &Service,
        request: FolderCompareRequest,
        display: DisplayOptions,
        expand_all: bool,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> FolderViewModel {
        let names = [
            request.left.to_string_lossy().into_owned(),
            request.right.to_string_lossy().into_owned(),
        ];
        let state = FolderViewModel::prepare(service, &request, &wake);
        FolderViewModel {
            request,
            titles,
            names,
            display,
            expand_all,
            expanded: HashSet::new(),
            collapsed: HashSet::new(),
            state,
            rows: Vec::new(),
            seen_generation: u64::MAX,
            rebuilt_at: None,
            selected: None,
            selected_index: None,
            first: 0,
            viewport: 40,
            scroll_to: None,
            events: Arc::new(AtomicU64::new(0)),
            wake,
            started: Instant::now(),
            finished_after: None,
            stats: RebuildStats::default(),
            rows_revision: 0,
            differences: Memo::new(None),
            difference_walks: Memo::new(0),
            show_log: false,
            log_height: LOG_HEIGHT,
            notes: Vec::new(),
        }
    }

    fn prepare(
        service: &Service,
        request: &FolderCompareRequest,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) -> FolderState {
        let request = request.clone();
        FolderState::Preparing(Job::spawn(
            service,
            JobKind::FolderCompare,
            "prepare folder comparison".into(),
            wake,
            move |svc, job| {
                let prepared = svc.folder_prepare(&request);
                // Stop while preparing: no scan may start (the UI also drops this job's result).
                if job.cancel.is_cancelled() {
                    return Err(ApiError::Cancelled);
                }
                prepared
            },
        ))
    }

    fn run(&mut self, service: &Service, prepared: PreparedFolderCompare) {
        let (observer, events) = ChannelObserver::bounded(EVENT_CAPACITY);
        // The observer blocks the scan while its channel is full (ARCH-082): drain it on a thread of its own,
        // counting the events and waking the UI at most once per rebuild interval.
        let counter = Arc::clone(&self.events);
        let wake = Arc::clone(&self.wake);
        let _ = thread::Builder::new()
            .name("hikasa-gui-scan-events".into())
            .spawn(move || {
                let mut woke = Instant::now();
                while let Ok(_event) = events.recv() {
                    counter.fetch_add(1, Ordering::Relaxed);
                    if woke.elapsed() >= REBUILD_INTERVAL {
                        woke = Instant::now();
                        wake();
                    }
                }
                wake();
            });
        let job_prepared = prepared.clone();
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        let job = Job::spawn(
            service,
            JobKind::FolderCompare,
            title,
            &self.wake,
            move |svc, job| {
                let result = svc.folder_run(&job_prepared, job, &observer);
                drop(observer);
                result
            },
        );
        self.display.ignore_unimportant |= prepared.ignore_unimportant();
        self.state = FolderState::Running { prepared, job };
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&self.names[0], &self.names[1]],
        )
    }

    /// The prepared comparison (its live model), once `folder_prepare` returned.
    pub fn prepared(&self) -> Option<&PreparedFolderCompare> {
        match &self.state {
            FolderState::Preparing(_) => None,
            FolderState::Running { prepared, .. } | FolderState::Done { prepared, .. } => {
                Some(prepared)
            }
            FolderState::Failed { prepared, .. } | FolderState::Stopped { prepared } => {
                prepared.as_ref()
            }
        }
    }

    /// The finished response.
    #[cfg(test)]
    pub fn response(&self) -> Option<&FolderCompareResponse> {
        match &self.state {
            FolderState::Done { response, .. } => Some(response),
            _ => None,
        }
    }

    /// True while a job of the view has ended but its result is not published yet (the window looks again at
    /// once).
    pub fn settling(&self) -> bool {
        match &self.state {
            FolderState::Preparing(job) => job.settling(),
            FolderState::Running { job, .. } => job.settling(),
            _ => false,
        }
    }

    /// True while a job of the view runs.
    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            FolderState::Preparing(_) | FolderState::Running { .. }
        )
    }

    /// Stops the comparison (the model stays consistent, unscanned folders stay unknown, §5.3). Stopped while
    /// preparing, the view is stopped at once and no scan starts (§1.10, §3.7 Stop).
    pub fn cancel(&mut self) {
        match &self.state {
            FolderState::Preparing(job) => {
                job.cancel();
                self.state = FolderState::Stopped { prepared: None };
                self.finished_after = Some(self.started.elapsed());
            }
            FolderState::Running { job, .. } => job.cancel(),
            _ => {}
        }
    }

    /// True once the view was stopped before its scan returned a result.
    #[cfg(test)]
    pub fn stopped(&self) -> bool {
        matches!(self.state, FolderState::Stopped { .. })
    }

    /// The rebuild timings.
    pub fn rebuild_stats(&self) -> &RebuildStats {
        &self.stats
    }

    /// Visible rows.
    #[cfg(test)]
    pub fn rows(&self) -> &[VisibleRow] {
        &self.rows
    }

    /// Number of visible rows (the tree's scroll height in rows).
    pub fn visible_count(&self) -> u32 {
        self.rows.len() as u32
    }

    /// Scan events received.
    pub fn events(&self) -> u64 {
        self.events.load(Ordering::Relaxed)
    }

    /// Time from the start to the end of the scan.
    pub fn finished_after(&self) -> Option<Duration> {
        self.finished_after
    }

    /// Polls the jobs and the model (once per frame); true when the tree changed.
    pub fn poll(&mut self, service: &Service) -> bool {
        let mut changed = false;
        match &self.state {
            FolderState::Preparing(job) => {
                if let Some(result) = job.try_result() {
                    changed = true;
                    match result {
                        Ok(prepared) => self.run(service, prepared),
                        Err(ApiError::Cancelled) => {
                            self.state = FolderState::Stopped { prepared: None };
                        }
                        Err(e) => {
                            self.notes.push(LogNote {
                                kind: LogKind::Error,
                                text: e.to_string(),
                            });
                            self.state = FolderState::Failed {
                                prepared: None,
                                error: e.to_string(),
                            };
                        }
                    }
                }
            }
            FolderState::Running { prepared, job } => {
                if let Some(result) = job.try_result() {
                    changed = true;
                    self.finished_after = Some(self.started.elapsed());
                    let prepared = prepared.clone();
                    let ms = self.finished_after.map_or(0, |d| d.as_millis());
                    self.state = match result {
                        Ok(response) => {
                            let verdict =
                                prepared.view_verdict(self.display.ignore_unimportant, false);
                            self.notes.push(LogNote {
                                kind: LogKind::Summary,
                                text: format!("{}, compared in {ms} ms.", verdict_text(verdict)),
                            });
                            for w in &response.warnings {
                                self.notes.push(LogNote {
                                    kind: LogKind::Warning,
                                    text: format!("[{}] {}", w.code, w.message),
                                });
                            }
                            FolderState::Done {
                                prepared,
                                response: Box::new(response),
                            }
                        }
                        Err(ApiError::Cancelled) => {
                            self.notes.push(LogNote {
                                kind: LogKind::Summary,
                                text: format!("Comparison stopped after {ms} ms."),
                            });
                            FolderState::Stopped {
                                prepared: Some(prepared),
                            }
                        }
                        Err(e) => {
                            self.notes.push(LogNote {
                                kind: LogKind::Error,
                                text: e.to_string(),
                            });
                            FolderState::Failed {
                                prepared: Some(prepared),
                                error: e.to_string(),
                            }
                        }
                    };
                }
            }
            _ => {}
        }
        if let Some(prepared) = self.prepared() {
            let generation = prepared.model().generation();
            let due = !self.busy()
                || self
                    .rebuilt_at
                    .is_none_or(|at| at.elapsed() >= REBUILD_INTERVAL);
            if (generation != self.seen_generation && due) || changed {
                self.rebuild();
                changed = true;
            }
        }
        changed
    }

    /// Rebuilds the visible rows from the model (O(visible), §1.6.4).
    pub fn rebuild(&mut self) {
        let began = Instant::now();
        let Some(prepared) = self.prepared() else {
            self.rows.clear();
            return;
        };
        let model = Arc::clone(prepared.model());
        let generation = model.generation();
        let read = model.read();
        let mut rows = std::mem::take(&mut self.rows);
        rows.clear();
        {
            let expand_all = self.expand_all;
            let expanded = &self.expanded;
            let collapsed = &self.collapsed;
            let is_open = |id: NodeId| {
                if expand_all {
                    !collapsed.contains(&id)
                } else {
                    expanded.contains(&id)
                }
            };
            read.visible_rows(&self.display, &is_open, &mut rows);
        }
        drop(read);
        self.rows = rows;
        self.rows_revision += 1;
        self.seen_generation = generation;
        self.rebuilt_at = Some(Instant::now());
        self.selected_index = self
            .selected
            .and_then(|sel| self.rows.iter().position(|r| r.node == sel));
        if self.selected_index.is_none() {
            self.selected = None;
        }
        let ms = began.elapsed().as_secs_f64() * 1000.0;
        if self.stats.samples.len() < 100_000 {
            self.stats.samples.push(ms);
        }
        self.stats.rows = self.rows.len();
        if self.busy() {
            self.stats.during_scan += 1;
        }
    }

    fn is_open(&self, id: NodeId) -> bool {
        if self.expand_all {
            !self.collapsed.contains(&id)
        } else {
            self.expanded.contains(&id)
        }
    }

    /// The scroll position changed: `first` visible row and visible row count. True when the rendered window
    /// moves.
    pub fn set_scroll(&mut self, first: u32, viewport: u32) -> bool {
        let moved = first != self.first || viewport != self.viewport;
        self.first = first;
        self.viewport = viewport.max(1);
        moved
    }

    /// Selects a visible row.
    pub fn select(&mut self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            self.selected = Some(row.node);
            self.selected_index = Some(index);
        }
    }

    /// Moves the selection by `delta` rows (arrow keys).
    pub fn move_selection(&mut self, delta: i64) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i64 - 1;
        let at = self
            .selected_index
            .map_or(if delta > 0 { -1 } else { last + 1 }, |i| i as i64);
        let index = (at + delta).clamp(0, last) as usize;
        self.select(index);
        self.scroll_to = Some(index as u32);
    }

    /// The selected node.
    #[cfg(test)]
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    /// Toggles a folder row (disclosure triangle, double-click).
    pub fn toggle(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).copied() else {
            return;
        };
        let open = self.is_open(row.node);
        self.set_open(row.node, !open);
        self.rebuild();
    }

    fn set_open(&mut self, id: NodeId, open: bool) {
        if self.expand_all {
            if open {
                self.collapsed.remove(&id);
            } else {
                self.collapsed.insert(id);
            }
        } else if open {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
    }

    /// The selected node as a folder: (id, open).
    fn selected_folder(&self) -> Option<(NodeId, bool)> {
        let id = self.selected?;
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let node = read.node(id)?;
        node.is_folder().then(|| (id, self.is_open(id)))
    }

    /// The selected file pair's specs (FC-062: both sides present, not folders).
    fn selected_pair(&self) -> Option<(NodeId, hikasa_core::Spec, hikasa_core::Spec)> {
        let id = self.selected?;
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let node = read.node(id)?;
        if node.is_folder() || node.left.is_none() || node.right.is_none() {
            return None;
        }
        Some((id, read.spec(id, Side::Left)?, read.spec(id, Side::Right)?))
    }

    /// The Text Compare request for the selected pair (FC-062): exact OS paths of both sides
    /// (`TextCompareRequest::for_specs`), the folder comparison's text rules and its ignore-unimportant.
    pub fn open_request(&self) -> Option<(TextCompareRequest, ViewOptions)> {
        let (_, left, right) = self.selected_pair()?;
        let mut request = TextCompareRequest::for_specs(&left, &right);
        request.rules_file.clone_from(&self.request.text_rules_file);
        request.rules_patch = self.request.text_patch.clone();
        request.ignore_unimportant = self.display.ignore_unimportant;
        Some((
            request,
            ViewOptions {
                filter: TextDisplayFilter::All,
                ignore_unimportant: self.display.ignore_unimportant,
                expanded: Vec::new(),
            },
        ))
    }

    /// The next/previous visible row whose item is not same (FC-061).
    fn difference_from(&self, forward: bool) -> Option<usize> {
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let iu = self.display.ignore_unimportant;
        let differs = |i: &usize| {
            self.rows
                .get(*i)
                .is_some_and(|r| read.category(r.node, iu).is_difference())
        };
        if forward {
            let from = self.selected_index.map_or(0, |i| i + 1);
            (from..self.rows.len()).find(differs)
        } else {
            let to = self.selected_index?;
            (0..to).rev().find(differs)
        }
    }

    /// Next/previous difference available: memoised per (rows revision, selection, ignore-unimportant), so
    /// renders and commands between two rebuilds do not walk the rows again.
    fn differences_around(&self) -> (bool, bool) {
        let key = (
            self.rows_revision,
            self.selected_index,
            self.display.ignore_unimportant,
        );
        if let Some(memo) = self.differences.get()
            && memo.key == key
        {
            return (memo.next, memo.prev);
        }
        self.difference_walks.set(self.difference_walks.get() + 1);
        let next = self.difference_from(true).is_some();
        let prev = self.difference_from(false).is_some();
        self.differences
            .set(Some(DifferenceMemo { key, next, prev }));
        (next, prev)
    }

    /// How often the next/previous-difference availability was computed (tests).
    #[cfg(test)]
    pub fn difference_walks(&self) -> u64 {
        self.difference_walks.get()
    }

    /// Fills the command context of this view.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.busy = self.busy();
        ctx.ready = self.prepared().is_some();
        if let Some((_, open)) = self.selected_folder() {
            ctx.open_folder_selected = open;
            ctx.closed_folder_selected = !open;
        }
        ctx.pair_selected = self.selected_pair().is_some();
        (ctx.next_difference, ctx.prev_difference) = self.differences_around();
    }

    /// The check state of a radio or toggle command (`None`: not a checkable command of this view).
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        let filter = self.display.filter;
        let toggles = filter.toggles();
        Some(match id {
            CommandId::DirShowAll => self.display.folders == FolderMode::AlwaysShowFolders,
            CommandId::DirCompareStructure => self.display.folders == FolderMode::CompareStructure,
            CommandId::DirCompareFilesOnly => self.display.folders == FolderMode::OnlyFiles,
            CommandId::InsigIgnored => self.display.ignore_unimportant,
            CommandId::SuppressFilters => self.display.suppress_filters,
            CommandId::ShowLog => self.show_log,
            _ => {
                if let Some(preset) = preset_of(id) {
                    filter == preset
                } else {
                    toggles.contains(toggle_of(id)?)
                }
            }
        })
    }

    /// Executes a command of this view. Returns `Some(open)` when the command was handled; `open` carries the
    /// Text Compare request of `Open` (FC-062).
    pub fn execute(
        &mut self,
        service: &Service,
        id: CommandId,
    ) -> Option<Option<(TextCompareRequest, ViewOptions)>> {
        match id {
            CommandId::Stop => self.cancel(),
            CommandId::FullRefresh => {
                self.cancel();
                self.state = FolderViewModel::prepare(service, &self.request, &self.wake);
                self.rows.clear();
                self.seen_generation = u64::MAX;
                self.started = Instant::now();
                self.finished_after = None;
                self.events.store(0, Ordering::Relaxed);
            }
            CommandId::Expand | CommandId::Collapse => {
                if let Some((node, _)) = self.selected_folder() {
                    self.set_open(node, id == CommandId::Expand);
                    self.rebuild();
                }
            }
            CommandId::ExpandSubfolders | CommandId::CollapseSubfolders => {
                if let Some((node, _)) = self.selected_folder() {
                    let open = id == CommandId::ExpandSubfolders;
                    for folder in self.subfolders(node) {
                        self.set_open(folder, open);
                    }
                    self.rebuild();
                }
            }
            CommandId::ExpandAll => {
                self.expand_all = true;
                self.collapsed.clear();
                self.rebuild();
            }
            CommandId::CollapseAll => {
                self.expand_all = false;
                self.expanded.clear();
                self.collapsed.clear();
                self.rebuild();
            }
            CommandId::Open => return Some(self.open_request()),
            CommandId::DiffNext | CommandId::DiffPrev => {
                if let Some(index) = self.difference_from(id == CommandId::DiffNext) {
                    self.select(index);
                    self.scroll_to = Some(index as u32);
                }
            }
            CommandId::DirShowAll => self.set_mode(FolderMode::AlwaysShowFolders),
            CommandId::DirCompareStructure => self.set_mode(FolderMode::CompareStructure),
            CommandId::DirCompareFilesOnly => self.set_mode(FolderMode::OnlyFiles),
            CommandId::InsigIgnored => {
                self.display.ignore_unimportant = !self.display.ignore_unimportant;
                self.rebuild();
            }
            CommandId::SuppressFilters => {
                self.display.suppress_filters = !self.display.suppress_filters;
                self.rebuild();
            }
            CommandId::ShowLog => self.show_log = !self.show_log,
            _ => {
                if let Some(preset) = preset_of(id) {
                    self.display.filter = preset;
                } else {
                    let bits = self.display.filter.toggles() ^ toggle_of(id)?;
                    self.display.filter = FolderDisplayFilter::Toggles { bits: bits.bits() };
                }
                self.rebuild();
            }
        }
        Some(None)
    }

    fn set_mode(&mut self, mode: FolderMode) {
        self.display.folders = mode;
        self.rebuild();
    }

    /// A folder and every folder below it.
    fn subfolders(&self, root: NodeId) -> Vec<NodeId> {
        let Some(prepared) = self.prepared() else {
            return Vec::new();
        };
        let read = prepared.model().read();
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if read.node(id).is_some_and(|n| n.is_folder()) {
                out.push(id);
                stack.extend(read.children(id));
            }
        }
        out
    }

    /// The D-23 outcome when the view closes (§1.8.7 items 22 and 37): the core's folder-view rule
    /// (`ViewOutcome::for_folder`: preparing, scanning or stopped is `Incomplete`, 100); a comparison that could
    /// not start or load its bases is `LoadFailed`.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        self.poll(service);
        match &self.state {
            FolderState::Failed { .. } => ViewOutcome::LoadFailed,
            _ => ViewOutcome::for_folder(
                self.prepared(),
                self.display.ignore_unimportant,
                self.busy(),
            ),
        }
    }

    /// The log panel is shown.
    pub fn show_log(&self) -> bool {
        self.show_log
    }

    /// The log panel's height (points).
    pub fn log_height(&self) -> f64 {
        self.log_height
    }

    /// Sets the log panel's height (the panel was resized).
    pub fn set_log_height(&mut self, height: f64) {
        if height.is_finite() {
            self.log_height = height.clamp(40.0, 4000.0);
        }
    }

    /// Applies the layout of the view type (D-108): `log_panel`, `log_height`.
    pub fn apply_layout(&mut self, layout: &State) {
        let view = crate::commands::Scope::Folder
            .view_name()
            .unwrap_or_default();
        if let Some(on) = layout
            .layout_value(view, "log_panel")
            .and_then(LayoutValue::as_bool)
        {
            self.show_log = on;
        }
        if let Some(h) = layout
            .layout_value(view, "log_height")
            .and_then(LayoutValue::as_float)
        {
            self.set_log_height(h);
        }
    }

    /// The layout keys this view owns (D-108): `log_panel`, `log_height`.
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("log_panel", LayoutValue::Bool(self.show_log)),
            ("log_height", LayoutValue::Float(self.log_height)),
        ]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, D-107), and the titles.
    pub fn duplicate(&self) -> (hikasa_session::api::ViewRequest, [Option<String>; 2]) {
        (
            hikasa_session::api::ViewRequest::Folder {
                request: self.request.clone(),
                display: self.display,
                expand_all: self.expand_all,
            },
            self.titles.clone(),
        )
    }

    /// The display options (the session's `display`, D-120).
    pub fn display_options(&self) -> &DisplayOptions {
        &self.display
    }

    /// The paths of a pair relative to the bases (TC-052): `/`-separated; `None` outside them.
    pub fn relative_pair(&self, request: &TextCompareRequest) -> Option<[String; 2]> {
        let relative = |base: &std::ffi::OsString, path: &std::ffi::OsString| {
            std::path::Path::new(path).strip_prefix(base).ok().map(|p| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
        };
        Some([
            relative(&self.request.left, &request.left)?,
            relative(&self.request.right, &request.right)?,
        ])
    }

    /// The bases as specs (the view list).
    pub fn specs(&self) -> Vec<String> {
        self.names.to_vec()
    }

    /// The running jobs (the job indicator, D-115).
    pub fn jobs(&self, view: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
        let job = match &self.state {
            FolderState::Preparing(job) => {
                let (phase, fraction) = job.progress();
                (job.title().to_owned(), phase, fraction)
            }
            FolderState::Running { job, .. } => {
                let (phase, fraction) = job.progress();
                (job.title().to_owned(), phase, fraction)
            }
            _ => return Vec::new(),
        };
        vec![JobStatus {
            view,
            title: job.0,
            phase: job.1,
            fraction: job.2,
        }]
    }

    /// The notes for the log since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    /// The status bar (D-115): the state on the left, the details on the right.
    pub fn status_items(&self) -> StatusItems {
        let status = self.status();
        StatusItems {
            left: status.state,
            right: if status.detail.is_empty() {
                Vec::new()
            } else {
                vec![status.detail]
            },
        }
    }

    /// The one-shot request to bring a row into view (selection moves).
    pub fn take_scroll(&mut self) -> Option<u32> {
        self.scroll_to.take()
    }

    /// The rows to render around the scroll position.
    pub fn window(&self) -> FolderWindow {
        let total = self.rows.len() as u32;
        let mut out = FolderWindow {
            total,
            rows: Vec::new(),
        };
        let Some(prepared) = self.prepared() else {
            return out;
        };
        let model = Arc::clone(prepared.model());
        let read = model.read();
        let start = self.first.saturating_sub(OVERSCAN).min(total);
        let end = (self.first + self.viewport + OVERSCAN).min(total);
        let iu = self.display.ignore_unimportant;
        for index in start..end {
            let Some(row) = self.rows.get(index as usize) else {
                break;
            };
            if let Some(view) = row_view(&read, row, index, iu, self.is_open(row.node)) {
                out.rows.push(RowView {
                    selected: Some(row.node) == self.selected,
                    ..view
                });
            }
        }
        out
    }

    /// Header and status bar texts.
    pub fn status(&self) -> FolderStatus {
        let mut status = FolderStatus {
            bases: [0, 1].map(|k| {
                crate::shared::shown_path(self.titles[k].as_deref().unwrap_or(&self.names[k]))
            }),
            ..FolderStatus::default()
        };
        match &self.state {
            FolderState::Failed { error, .. } => {
                status.error = Some(crate::shared::shown_path(error))
            }
            FolderState::Done { response, .. } if !response.warnings.is_empty() => {
                status.error = Some(crate::shared::shown_path(
                    &response
                        .warnings
                        .iter()
                        .map(|w| w.message.as_str())
                        .collect::<Vec<_>>()
                        .join(" · "),
                ));
            }
            _ => {}
        }
        let Some(prepared) = self.prepared() else {
            status.state = if self.busy() {
                "Preparing…".into()
            } else {
                "Stopped before the scan started.".into()
            };
            return status;
        };
        let read = prepared.model().read();
        let iu = self.display.ignore_unimportant;
        let stats = read.stats(iu);
        let f = &stats.files;
        let counts = format!(
            "{} same · {} different · {} left newer · {} right newer · {} left orphans · {} right orphans{}{}",
            f.same,
            f.different,
            f.left_newer,
            f.right_newer,
            f.left_orphan,
            f.right_orphan,
            if f.unknown > 0 {
                format!(" · {} unknown", f.unknown)
            } else {
                String::new()
            },
            if f.error > 0 {
                format!(" · {} errors", f.error)
            } else {
                String::new()
            },
        );
        drop(read);
        status.state = if self.busy() {
            format!("Scanning… {counts}")
        } else {
            let verdict = prepared.view_verdict(iu, false);
            format!("{} · {counts}", verdict_text(verdict))
        };
        status.detail = format!(
            "{} files, {} folders left · {} files, {} folders right · {} rows shown{}",
            stats.left.files,
            stats.left.folders,
            stats.right.files,
            stats.right.folders,
            self.rows.len(),
            self.finished_after.map_or(String::new(), |d| format!(
                " · compared in {} ms",
                d.as_millis()
            )),
        );
        status
    }
}

/// The display filter preset of a command.
fn preset_of(id: CommandId) -> Option<FolderDisplayFilter> {
    Some(match id {
        CommandId::FavCompareAll => FolderDisplayFilter::All,
        CommandId::FavDiff => FolderDisplayFilter::Differences,
        CommandId::FavSame => FolderDisplayFilter::Same,
        CommandId::FavNoOrphan => FolderDisplayFilter::NoOrphans,
        CommandId::FavDiffNoOrphan => FolderDisplayFilter::DifferencesNoOrphans,
        CommandId::FavOrphan => FolderDisplayFilter::Orphans,
        CommandId::FavLeftNewer => FolderDisplayFilter::LeftNewer,
        CommandId::FavRightNewer => FolderDisplayFilter::RightNewer,
        CommandId::FavLeftNewerOrphan => FolderDisplayFilter::LeftNewerAndLeftOrphans,
        CommandId::FavRightNewerOrphan => FolderDisplayFilter::RightNewerAndRightOrphans,
        CommandId::FavLeftOrphan => FolderDisplayFilter::LeftOrphans,
        CommandId::FavRightOrphan => FolderDisplayFilter::RightOrphans,
        CommandId::FavCompareNone => FolderDisplayFilter::None,
        _ => return None,
    })
}

/// The atomic toggle of a command (FC-055 toggle mode).
fn toggle_of(id: CommandId) -> Option<FolderToggles> {
    Some(match id {
        CommandId::TogSame => FolderToggles::SAME,
        CommandId::TogLeftOrphan => FolderToggles::LEFT_ORPHANS,
        CommandId::TogRightOrphan => FolderToggles::RIGHT_ORPHANS,
        CommandId::TogLeftNewer => FolderToggles::LEFT_NEWER,
        CommandId::TogRightNewer => FolderToggles::RIGHT_NEWER,
        CommandId::TogDiff => FolderToggles::DIFFERENCES,
        _ => return None,
    })
}

fn verdict_text(verdict: CompareVerdict) -> &'static str {
    match verdict {
        CompareVerdict::NoDifferences => "No differences",
        CompareVerdict::UnimportantOnly => "Only unimportant differences",
        CompareVerdict::Differences => "Differences",
        CompareVerdict::Incomplete => "Incomplete",
    }
}

/// Centre symbol and tooltip of a category.
fn mark(category: Category) -> (&'static str, &'static str) {
    match category {
        Category::Same => ("=", "Same"),
        Category::Different => ("≠", "Different"),
        Category::LeftNewer => ("◀", "Left newer"),
        Category::RightNewer => ("▶", "Right newer"),
        Category::LeftOrphan => ("◧", "Left orphan"),
        Category::RightOrphan => ("◨", "Right orphan"),
        Category::Unknown => ("?", "Unknown (not compared yet)"),
        Category::Error => ("!", "Error"),
        Category::Filtered => ("·", "Filtered"),
    }
}

fn row_view(
    read: &TreeRead<'_>,
    row: &VisibleRow,
    index: u32,
    iu: bool,
    open: bool,
) -> Option<RowView> {
    let node = read.node(row.node)?;
    let folder = node.is_folder();
    let totals = read.totals(row.node);
    let cell = |entry: Option<&EntryView<'_>>, side: Side| {
        entry.map(|e| {
            let bytes = if folder {
                if side == Side::Left {
                    totals.left.bytes
                } else {
                    totals.right.bytes
                }
            } else {
                e.size
            };
            Cell {
                name: hikasa_report::escape_name(e.name).into_owned(),
                size: if folder && node.scan != hikasa_folder::ScanStatus::Scanned {
                    String::new()
                } else {
                    size_text(bytes)
                },
                modified: e
                    .mtime
                    .map_or(String::new(), |t| time_text(&t.to_rfc3339())),
                colour: read.colour(row.node, side, iu),
            }
        })
    };
    let category = read.category(row.node, iu);
    let (mark, mark_title) = mark(category);
    let errors = read
        .errors(row.node)
        .iter()
        .map(|e| e.error.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    Some(RowView {
        index,
        node: row.node.0,
        depth: row.depth,
        folder,
        open,
        has_children: folder && node.scan == hikasa_folder::ScanStatus::Scanned,
        left: cell(node.left.as_ref(), Side::Left),
        right: cell(node.right.as_ref(), Side::Right),
        mark,
        mark_title,
        category,
        selected: false,
        errors,
    })
}

/// `1 234 567` (thin grouping, bytes).
pub fn size_text(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

/// `2024-05-01 12:00:07` from RFC 3339 UTC text.
pub fn time_text(rfc3339: &str) -> String {
    let civil: String = rfc3339.chars().take(19).collect();
    civil.replacen('T', " ", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_times() {
        assert_eq!(size_text(0), "0");
        assert_eq!(size_text(999), "999");
        assert_eq!(size_text(1000), "1\u{202f}000");
        assert_eq!(size_text(1_234_567), "1\u{202f}234\u{202f}567");
        assert_eq!(time_text("2024-05-01T12:00:07.5Z"), "2024-05-01 12:00:07");
    }

    #[test]
    fn every_preset_and_toggle_has_a_command() {
        let presets: Vec<_> = CommandId::ALL
            .iter()
            .filter_map(|&id| preset_of(id))
            .collect();
        assert_eq!(presets, FolderDisplayFilter::PRESETS);
        let bits = CommandId::ALL
            .iter()
            .filter_map(|&id| toggle_of(id))
            .fold(FolderToggles::empty(), |a, b| a | b);
        assert_eq!(bits, FolderToggles::all());
    }
}
