//! The Folder Compare view model (docs/04 §4–§7; contract §1.10, §3.7): the comparison runs as jobs
//! (`folder_prepare`, then `folder_run` into the live model), the tree is rebuilt from the model whenever its
//! generation changes (incremental results while scanning, D-55), and the window of rows on screen is turned
//! into drawing data. States, colours, visibility, statistics and verdicts are `TreeRead` calls.

use std::cell::Cell as Memo;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hikasa_core::{
    CompareVerdict, ContentMethod, ContentOutcome, ContentResult, JobKind, Pair, RelPath, Side,
};
use hikasa_folder::ops::{Confirmation, FolderOp, OpPlan, ReadOnlySides};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::{
    ArchiveHandling, Category, ChannelObserver, ColourClass, DisplayOptions, EntryView,
    FolderDisplayFilter, FolderMode, FolderModel, FolderToggles, NodeFlags, NodeId, NodeView,
    ScanStatus, TreeRead, VisibleRow, expanded_on_load,
};
use hikasa_session::api::{
    ExpandWords, FolderCompareRequest, FolderCompareResponse, FolderOpOutcome, FolderOpRequest,
    OpOverrides, OpRequest, PreparedFolderCompare, RefreshDto, SelectRequest, TextCompareRequest,
    ViewOutcome,
};
use hikasa_session::ipc::{OpenOptions, ViewId};
use hikasa_session::{ApiError, Service};
use hikasa_text::{TextDisplayFilter, ViewOptions};
use hikasa_vfs::ops::{ItemStamp, OverwritePolicy};

use crate::commands::{CommandId, Ctx};
use crate::dialog::DialogModel;
use crate::job::Job;
use crate::notify::NoticeKind;
use crate::op_dialogs::{DeleteChoice, DeleteDialog, PlanConfirmDialog};
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
    /// An archive by its name, closed or opened (the zipper icon, D-153).
    pub archive: bool,
    /// `Open` opens this closed archive as a folder (`OnceOpened`, FC-015).
    pub opens_archive: bool,
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

/// What a view may write (FC-003, CLI-003): the sides Disable editing protects, and `--nobackups`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WritePolicy {
    /// The read-only sides (`--ro`, `--ro1`, `--ro2`, IPC `ReadOnly`, a session's `[specs] read_only_*`).
    pub read_only: ReadOnlySides,
    /// `--nobackups`: the view's operations keep no backups, whatever `[backups]` says.
    pub no_backups: bool,
}

impl WritePolicy {
    /// The policy of an `OpenView` (IPC, the command line): `--ro` is both sides.
    pub fn of_open(options: &OpenOptions) -> WritePolicy {
        WritePolicy {
            read_only: ReadOnlySides {
                left: options.read_only.all || options.read_only.left,
                right: options.read_only.all || options.read_only.right,
            },
            no_backups: options.no_backups,
        }
    }

    /// The overrides of the view's operations: `--nobackups` is `backup = Some(false)` (§1.6.20).
    pub fn overrides(&self) -> OpOverrides {
        OpOverrides {
            backup: self.no_backups.then_some(false),
            ..OpOverrides::default()
        }
    }

    /// True when the view may write `side`.
    pub fn writable(&self, side: Side) -> bool {
        !self.read_only.get(side)
    }
}

/// What a view hands to its window between two polls (M2 W6d): a dialog to show, a notice, an operation's outcome
/// for the log (D-116, D-117).
pub enum FolderEvent {
    /// A dialog of the view's operation.
    Dialog(Box<dyn DialogModel>),
    /// A notice (and its log entry).
    Notice {
        /// Its kind.
        kind: NoticeKind,
        /// Its text.
        text: String,
    },
    /// An operation ended: the log takes its entries (`op_log_entries`), the window one notice.
    Outcome {
        /// The command's caption.
        caption: String,
        /// What ran.
        outcome: Box<FolderOpOutcome>,
        /// How long it ran.
        elapsed: Duration,
    },
}

/// The row a child view was opened for (FC-029(c)): its path and the stamps of both sides at `Open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildTicket {
    /// The row's path: its result goes to the file row there when it comes (a refresh may have given the row a new
    /// record; the stamps decide whether it is the same pair).
    pub path: RelPath,
    /// Size and time of each side when the child opened.
    pub stamps: Pair<ItemStamp>,
}

/// The Text Compare view `Open` asks for (FC-062), with its row (FC-029(c)).
pub struct OpenPair {
    /// The request.
    pub request: TextCompareRequest,
    /// The view options.
    pub view: ViewOptions,
    /// The row it compares.
    pub ticket: Option<ChildTicket>,
}

/// The model and its generation when a plan was made: a plan runs only on the model it was made from, unchanged.
#[derive(Clone)]
struct Mark {
    model: Arc<FolderModel>,
    generation: u64,
}

impl Mark {
    fn of(prepared: &PreparedFolderCompare) -> Mark {
        Mark {
            model: Arc::clone(prepared.model()),
            generation: prepared.model().generation(),
        }
    }

    /// True while the view still shows this model and nothing changed it.
    fn holds(&self, prepared: Option<&PreparedFolderCompare>) -> bool {
        prepared.is_some_and(|p| {
            Arc::ptr_eq(p.model(), &self.model) && p.model().generation() == self.generation
        })
    }
}

/// What a refresh of the view is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AfterRefresh {
    /// The rows an operation wrote.
    Operation,
    /// A folder Expand opened that was not scanned (FC-012): it opens.
    Expand(NodeId),
    /// A closed archive `Open` opened (§1.6.20): once it opened, the view's session keeps its path and its row opens.
    Archive(String),
}

/// The operation or refresh of a view (M2 W6d): one at a time, and none while the comparison runs.
enum Task {
    /// The Delete dialog is open.
    Choosing {
        command: CommandId,
        caption: String,
        node: NodeId,
        mark: Mark,
    },
    /// `folder_op_plan` and `folder_confirmation` (both read the disk: a job, D-105).
    Planning {
        caption: String,
        view: ViewId,
        mark: Mark,
        /// The row the command was given on: the question is asked only while it is still the selected one.
        node: NodeId,
        /// The row and its sides as the question names them.
        subject: String,
        job: Job<(OpPlan, Confirmation)>,
    },
    /// The confirmation dialog is open.
    Asking {
        caption: String,
        plan: Box<OpPlan>,
        mark: Mark,
    },
    /// `folder_op_execute` (`JobKind::FolderOp`; Stop cancels it, D-115).
    Executing {
        caption: String,
        mark: Mark,
        refresh: Vec<NodeId>,
        began: Instant,
        job: Job<FolderOpOutcome>,
    },
    /// `folder_refresh` or `folder_open_archive`.
    Refreshing {
        after: AfterRefresh,
        model: Arc<FolderModel>,
        /// The open folders and the selected row by path when it started: found again only where the user changed
        /// nothing while it ran.
        points: Option<Restore>,
        job: Job<RefreshDto>,
    },
}

impl Task {
    /// The job of the task, while one runs.
    fn job_status(&self) -> Option<(String, Option<hikasa_core::Phase>, Option<f32>)> {
        let (title, (phase, fraction)) = match self {
            Task::Planning { job, .. } => (job.title(), job.progress()),
            Task::Executing { job, .. } => (job.title(), job.progress()),
            Task::Refreshing { job, .. } => (job.title(), job.progress()),
            Task::Choosing { .. } | Task::Asking { .. } => return None,
        };
        Some((title.to_owned(), phase, fraction))
    }

    fn settling(&self) -> bool {
        match self {
            Task::Planning { job, .. } => job.settling(),
            Task::Executing { job, .. } => job.settling(),
            Task::Refreshing { job, .. } => job.settling(),
            Task::Choosing { .. } | Task::Asking { .. } => false,
        }
    }

    fn cancel(&self) {
        match self {
            Task::Planning { job, .. } => job.cancel(),
            Task::Executing { job, .. } => job.cancel(),
            Task::Refreshing { job, .. } => job.cancel(),
            Task::Choosing { .. } | Task::Asking { .. } => {}
        }
    }
}

/// The open folders and the selected row by path, kept across a refresh or a full re-run (ids move, records are
/// replaced).
#[derive(Debug, Clone, Default)]
struct Restore {
    /// `expanded` (or `collapsed` with expand-all), by path, with the id each had when this was taken.
    folders: Vec<(String, NodeId)>,
    /// The selected row, and whether it is a folder (a file and a folder of one name are two rows, D-20).
    selected: Option<(String, bool)>,
    /// [`FolderViewModel::selection_moves`] when this was taken: a selection the user made since wins.
    moves: u64,
}

/// An operation's job that outlives its view (the tab closed, the view was replaced, the window quits): cancelled, it
/// ends after its current item, and the window reports what it did (§1.6.25 W6d after review).
pub struct RunningOp {
    /// The command's caption.
    pub caption: String,
    /// When it started.
    pub began: Instant,
    /// The job.
    pub job: Job<FolderOpOutcome>,
}

/// What a view hands over when it ends: the events not taken yet (an outcome among them) and its running operation.
pub struct Retired {
    /// Dialogs, notices and outcomes the window did not take yet.
    pub events: Vec<FolderEvent>,
    /// The operation still writing, cancelled.
    pub running: Option<RunningOp>,
}

/// A gate the tests hold a view's operation jobs at (deterministic Stop and busy states, §6.6 "no timers").
#[cfg(test)]
pub type Gate = Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>;

#[cfg(test)]
fn wait_at(gate: Option<&Gate>) {
    if let Some(gate) = gate {
        let (open, turn) = &**gate;
        let mut open = open.lock().expect("gate");
        while !*open {
            open = turn.wait(open).expect("gate");
        }
    }
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
    /// An archive `Open` just added: its folder row is shown expanded when it appears (D-153).
    expand_archive: Option<String>,
    /// What the view may write (FC-003, CLI-003).
    write: WritePolicy,
    /// The operation or refresh of the view (M2 W6d).
    task: Option<Task>,
    /// What the window takes at its next poll.
    outbox: Vec<FolderEvent>,
    /// The open folders and selection to find again after a full re-run.
    restore: Option<Restore>,
    /// FC-012: the folders `expanded_on_load` names were opened for the current model.
    loaded: bool,
    /// FC-029(c): the child views and their rows.
    children: HashMap<ViewId, ChildTicket>,
    /// Child results that wait for the view's task to end.
    child_results: Vec<(ViewId, ContentOutcome)>,
    /// A folder the tree opened that was not scanned: refreshed at the next poll (FC-012).
    unscanned_open: Option<NodeId>,
    /// Folders an Expand asked for while a task ran, by path: refreshed and opened once it ends (FC-012).
    queued_expand: Vec<String>,
    /// Bumped by every selection the user makes: a refresh does not put back a selection older than one made while it
    /// ran.
    selection_moves: u64,
    /// Tests hold the operation and refresh jobs here.
    #[cfg(test)]
    pub gate: Option<Gate>,
    /// Tests hold a comparison here once it scanned, before it reports (a view that is still comparing).
    #[cfg(test)]
    pub scan_gate: Option<Gate>,
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
            expand_archive: None,
            write: WritePolicy::default(),
            task: None,
            outbox: Vec::new(),
            restore: None,
            loaded: false,
            children: HashMap::new(),
            child_results: Vec::new(),
            unscanned_open: None,
            queued_expand: Vec::new(),
            selection_moves: 0,
            #[cfg(test)]
            gate: None,
            #[cfg(test)]
            scan_gate: None,
        }
    }

    /// Sets what the view may write (FC-003, CLI-003).
    pub fn set_write_policy(&mut self, write: WritePolicy) {
        self.write = write;
    }

    /// What the view may write.
    pub fn write_policy(&self) -> WritePolicy {
        self.write
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
                let prepared = svc.folder_prepare_in(&request, job);
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
        #[cfg(test)]
        let gate = self.scan_gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderCompare,
            title,
            &self.wake,
            move |svc, job| {
                let result = svc.folder_run(&job_prepared, job, &observer);
                drop(observer);
                #[cfg(test)]
                wait_at(gate.as_ref());
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
        self.task.as_ref().is_some_and(Task::settling)
            || match &self.state {
                FolderState::Preparing(job) => job.settling(),
                FolderState::Running { job, .. } => job.settling(),
                _ => false,
            }
    }

    /// True while the comparison runs (preparing or scanning).
    pub fn scanning(&self) -> bool {
        matches!(
            self.state,
            FolderState::Preparing(_) | FolderState::Running { .. }
        )
    }

    /// True while a job of the view runs: the comparison, or an operation's or a refresh's job (not a dialog that
    /// waits for its answer).
    pub fn busy(&self) -> bool {
        self.scanning() || self.task.as_ref().is_some_and(|t| t.job_status().is_some())
    }

    /// Stops the comparison (the model stays consistent, unscanned folders stay unknown, §5.3). Stopped while
    /// preparing, the view is stopped at once and no scan starts (§1.10, §3.7 Stop). An operation's job is cancelled
    /// (D-115): what ran stays, what did not is reported, and the rows are refreshed.
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
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

    /// The view ends (its tab closes, its model is replaced, the window quits): everything stops, and what the window
    /// must still report comes back: the events it did not take, and an operation's job, cancelled, which ends after
    /// its current item (the cancel is cooperative). A plan or a question that had not run writes nothing.
    pub fn retire(&mut self) -> Retired {
        let running = match self.task.take() {
            Some(Task::Executing {
                caption,
                began,
                job,
                ..
            }) => {
                job.cancel();
                Some(RunningOp {
                    caption,
                    began,
                    job,
                })
            }
            Some(task) => {
                task.cancel();
                None
            }
            None => None,
        };
        self.cancel();
        Retired {
            events: std::mem::take(&mut self.outbox),
            running,
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
        if changed && !self.scanning() && !self.loaded {
            self.after_run();
        }
        if let Some(node) = self.unscanned_open.take() {
            self.expand(service, node);
            changed = true;
        }
        changed |= self.poll_task(service);
        if self.task.is_none() && !self.scanning() && !self.queued_expand.is_empty() {
            changed |= self.run_queued_expands(service);
        }
        if self.task.is_none() && !self.child_results.is_empty() {
            self.apply_child_results();
            changed = true;
        }
        if let Some(prepared) = self.prepared() {
            let generation = prepared.model().generation();
            let due = !self.scanning()
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
        self.open_pending_archive();
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
        if self.scanning() {
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
            self.selection_moves += 1;
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

    /// Toggles a folder row (disclosure triangle, double-click). A folder that was not scanned is refreshed at the
    /// next poll and opens then (FC-012).
    pub fn toggle(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).copied() else {
            return;
        };
        let open = self.is_open(row.node);
        if !open && self.not_scanned(row.node) {
            self.unscanned_open = Some(row.node);
            return;
        }
        self.set_open(row.node, !open);
        self.rebuild();
    }

    /// True for a folder row that was not listed (auto-scan off, or a stop).
    fn not_scanned(&self, id: NodeId) -> bool {
        self.prepared().is_some_and(|p| {
            p.model()
                .read()
                .node(id)
                .is_some_and(|n| n.is_folder() && n.scan == ScanStatus::NotScanned)
        })
    }

    /// Opens a folder; one that was not scanned is refreshed first, and opens when its rows are in (FC-012). While a
    /// task of the view runs, the Expand waits for it, by path (the task may move the ids).
    fn expand(&mut self, service: &Service, node: NodeId) {
        if self.not_scanned(node) && !self.scanning() {
            if self.task.is_none() {
                self.refresh_rows(service, vec![node], AfterRefresh::Expand(node));
            } else if let Some(path) = self.path_of(node)
                && !self.queued_expand.contains(&path)
            {
                self.queued_expand.push(path);
            }
            return;
        }
        self.set_open(node, true);
        self.rebuild();
    }

    /// The path of a row of the current model.
    fn path_of(&self, id: NodeId) -> Option<String> {
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        read.node(id)?;
        Some(read.path(id).as_str().to_owned())
    }

    /// The Expands that waited for a task, now that none runs: each folder still there is expanded (the first that
    /// needs a refresh starts it; the rest wait for that one).
    fn run_queued_expands(&mut self, service: &Service) -> bool {
        let mut changed = false;
        for path in std::mem::take(&mut self.queued_expand) {
            let found = self.prepared().and_then(|p| {
                let read = p.model().read();
                live_at(&read, &path, true)
            });
            if let Some(node) = found {
                self.expand(service, node);
                changed = true;
            }
        }
        changed
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

    /// The selected row and its path when `Open` opens it as a folder: a closed archive with `OnceOpened`.
    fn selected_closed_archive(&self) -> Option<(NodeId, String)> {
        let id = self.selected?;
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let node = read.node(id)?;
        opens_archive(&read, &node).then(|| (id, read.path(id).as_str().to_owned()))
    }

    /// Opens the archive in place with `folder_open_archive`: a refresh of its folder, not a new comparison (§1.6.20).
    /// Once it opened, `path` joins the view's `rules.open_archives`, so the session keeps it; an open that failed or
    /// was stopped leaves both as they were.
    fn open_archive(&mut self, service: &Service, node: NodeId, path: String) {
        if self.task.is_some() {
            return;
        }
        let Some(prepared) = self.prepared() else {
            return;
        };
        let model = Arc::clone(prepared.model());
        let job_model = Arc::clone(&model);
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderRefresh,
            "open the archive".into(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                wait_at(gate.as_ref());
                svc.folder_open_archive(&job_model, node, job, &hikasa_folder::NullObserver)
            },
        );
        let points = self.restore_points();
        self.task = Some(Task::Refreshing {
            after: AfterRefresh::Archive(path),
            model,
            points,
            job,
        });
    }

    /// An archive opened: the view's session keeps its path, and its row opens once it is a folder.
    fn archive_opened(&mut self, path: String) {
        let Some(prepared) = self.prepared() else {
            return;
        };
        let mut open = self
            .request
            .patch
            .open_archives
            .clone()
            .unwrap_or_else(|| prepared.rules().open_archives.clone());
        if !open.contains(&path) {
            open.push(path.clone());
        }
        self.request.patch.open_archives = Some(open);
        self.expand_archive = Some(path);
    }

    /// The paths of `rules.open_archives` the view adds to its comparison (what its session keeps).
    #[cfg(test)]
    pub fn open_archives(&self) -> Option<&[String]> {
        self.request.patch.open_archives.as_deref()
    }

    /// Prepares and runs the comparison again; the open folders and the selected row are found again by path once
    /// it has run (its ids are new).
    fn refresh(&mut self, service: &Service) {
        if let Some(task) = self.task.take() {
            task.cancel();
        }
        let restore = self.restore_points();
        self.cancel();
        if let Some(restore) = restore {
            self.restore = Some(restore);
        }
        // An id of the old model names nothing in the new one (an Expand that waits for a task does so by path).
        self.unscanned_open = None;
        self.expanded.clear();
        self.collapsed.clear();
        self.selected = None;
        self.selected_index = None;
        self.children.clear();
        self.child_results.clear();
        self.loaded = false;
        self.state = FolderViewModel::prepare(service, &self.request, &self.wake);
        self.rows.clear();
        self.seen_generation = u64::MAX;
        self.started = Instant::now();
        self.finished_after = None;
        self.events.store(0, Ordering::Relaxed);
    }

    /// The open folders and the selected row by path (`None` without a model).
    fn restore_points(&self) -> Option<Restore> {
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let set = if self.expand_all {
            &self.collapsed
        } else {
            &self.expanded
        };
        let mut folders: Vec<(String, NodeId)> = set
            .iter()
            .filter(|&&id| read.node(id).is_some_and(|n| n.is_folder()))
            .map(|&id| (read.path(id).as_str().to_owned(), id))
            .collect();
        folders.sort();
        Some(Restore {
            folders,
            selected: self.selected.map(|id| {
                let folder = read.node(id).is_some_and(|n| n.is_folder());
                (read.path(id).as_str().to_owned(), folder)
            }),
            moves: self.selection_moves,
        })
    }

    /// What of `points` the user left as it was while a refresh ran: the folders still open (one closed meanwhile
    /// stays closed; with expand-all, still closed) and the selection, unless another row was selected meanwhile (a click during a refresh wins).
    /// Read before the refresh's `renumbered` moves the ids.
    fn untouched(&self, points: Restore) -> Restore {
        // The set the points were taken from: the open folders, or the closed ones with expand-all.
        let set = if self.expand_all {
            &self.collapsed
        } else {
            &self.expanded
        };
        let still_open = |id: NodeId| set.contains(&id);
        Restore {
            folders: points
                .folders
                .into_iter()
                .filter(|(_, id)| still_open(*id))
                .collect(),
            selected: points
                .selected
                .filter(|_| self.selection_moves == points.moves),
            moves: points.moves,
        }
    }

    /// A run ended: the open folders of a re-run are found again by path; a first run opens what FC-012 asks
    /// (`expanded_on_load`: every folder, or those with a difference below).
    fn after_run(&mut self) {
        self.loaded = true;
        let Some(prepared) = self.prepared() else {
            return;
        };
        let model = Arc::clone(prepared.model());
        let read = model.read();
        match self.restore.take() {
            Some(restore) => {
                // A file and a folder of one name are two rows (D-20): the folder.
                let found: Vec<NodeId> = restore
                    .folders
                    .iter()
                    .filter_map(|(p, _)| live_at(&read, p, true))
                    .collect();
                if self.expand_all {
                    self.collapsed.extend(found);
                } else {
                    self.expanded.extend(found);
                }
                // A row the user selected while the comparison ran again stays selected.
                if self.selection_moves == restore.moves {
                    self.selected = restore
                        .selected
                        .and_then(|(path, folder)| live_at(&read, &path, folder));
                }
            }
            None => {
                if !self.expand_all {
                    let open = expanded_on_load(&read, &read.rules().handling);
                    self.expanded.extend(open);
                }
            }
        }
        drop(read);
        self.rebuild();
    }

    // ---- operations (M2 W6d, §1.6.20) ------------------------------------------------------------------------------

    /// What the window takes since the last call: dialogs, notices, outcomes.
    pub fn take_events(&mut self) -> Vec<FolderEvent> {
        std::mem::take(&mut self.outbox)
    }

    /// The selected row with what it holds per side, while the view may start an operation: a finished comparison,
    /// no job of the view and no operation under way.
    fn operable_row(&self) -> Option<(NodeId, [bool; 2])> {
        if self.busy() || self.task.is_some() {
            return None;
        }
        let id = self.selected?;
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let node = read.node(id)?;
        (id != NodeId::ROOT).then_some((id, [node.left.is_some(), node.right.is_some()]))
    }

    /// Starts `command` (`CopyRight`, `CopyLeft`, `Delete`, `PermanentDelete`) on the selected row: a copy plans at
    /// once, a delete first asks its sides in the dialog returned. `caption` is the command's; `view` the view's id;
    /// `require_side` is `require_side_selection_in_delete`. Nothing starts while an operation is under way, for a
    /// side the view may not write, or without a selected row: the window's predicates say the same.
    pub fn start_operation(
        &mut self,
        service: &Service,
        view: ViewId,
        command: CommandId,
        caption: &str,
        require_side: bool,
    ) -> Option<Box<dyn DialogModel>> {
        let (node, on) = self.operable_row()?;
        let mark = Mark::of(self.prepared()?);
        match command {
            CommandId::CopyRight | CommandId::CopyLeft => {
                let to = if command == CommandId::CopyRight {
                    Side::Right
                } else {
                    Side::Left
                };
                let from = to.opposite();
                if !self.write.writable(to) || !on[usize::from(from == Side::Right)] {
                    return None;
                }
                let op = FolderOp::Copy {
                    to,
                    overwrite: OverwritePolicy::Always,
                };
                let subject = self.subject(node, SideSet::of(from), Some(to));
                self.plan(
                    service,
                    view,
                    caption,
                    op,
                    node,
                    SideSet::of(from),
                    mark,
                    subject,
                );
                None
            }
            CommandId::Delete | CommandId::PermanentDelete => {
                let writable = [
                    self.write.writable(Side::Left),
                    self.write.writable(Side::Right),
                ];
                if !(0..2).any(|k| on[k] && writable[k]) {
                    return None;
                }
                let (path, holds) = {
                    let read = mark.model.read();
                    let totals = read.totals(node);
                    let view_of = read.node(node)?;
                    let folder = view_of.is_folder();
                    let hold = |entry: Option<&EntryView<'_>>, t: &hikasa_folder::SideTotals| {
                        entry.map(|e| {
                            if folder {
                                format!(
                                    "a folder: {} files, {} folders, {} bytes below it",
                                    t.files, t.folders, t.bytes
                                )
                            } else {
                                format!("a {}: {} bytes", kind_word(e.kind), e.size)
                            }
                        })
                    };
                    (
                        read.path(node).as_str().to_owned(),
                        [
                            hold(view_of.left.as_ref(), &totals.left),
                            hold(view_of.right.as_ref(), &totals.right),
                        ],
                    )
                };
                let permanent = command == CommandId::PermanentDelete;
                let dialog =
                    DeleteDialog::new(view, path, holds, writable, permanent, require_side);
                self.task = Some(Task::Choosing {
                    command,
                    caption: caption.to_owned(),
                    node,
                    mark,
                });
                Some(Box::new(dialog))
            }
            _ => None,
        }
    }

    /// The Delete dialog closed: with a choice the plan is made, without one nothing happens.
    pub fn delete_chosen(&mut self, service: &Service, view: ViewId, choice: Option<DeleteChoice>) {
        let Some(Task::Choosing {
            command,
            caption,
            node,
            mark,
        }) = self.task.take()
        else {
            return;
        };
        let Some(choice) = choice else {
            return;
        };
        if !mark.holds(self.prepared()) {
            self.refuse_changed(&caption);
            return;
        }
        if self.selected != Some(node) {
            self.refuse_moved(&caption);
            return;
        }
        let permanent = command == CommandId::PermanentDelete;
        let recycle = if permanent {
            hikasa_folder::ops::Recycle::default()
        } else {
            choice.recycle
        };
        let op = FolderOp::Delete {
            sides: choice.sides,
            recycle,
        };
        let subject = self.subject(node, choice.sides, None);
        self.plan(
            service,
            view,
            &caption,
            op,
            node,
            choice.sides,
            mark,
            subject,
        );
    }

    /// The row an operation acts on and its sides, as its question names them: `a/b.txt: from the left to the right`,
    /// `a/b.txt: on the left`, `a/b.txt: on both sides`.
    fn subject(&self, node: NodeId, sides: SideSet, to: Option<Side>) -> String {
        let path = self.path_of(node).unwrap_or_default();
        let where_ = match to {
            Some(Side::Left) => "from the right to the left",
            Some(_) => "from the left to the right",
            None if sides.contains(SideSet::LEFT) && sides.contains(SideSet::RIGHT) => {
                "on both sides"
            }
            None if sides.contains(SideSet::LEFT) => "on the left",
            None => "on the right",
        };
        format!("{path}: {where_}")
    }

    /// Plans `op` on the row `node` (its `sides`) in a job: `folder_op_plan`, then `folder_confirmation`, which reads
    /// the options. Disable editing and `--nobackups` go with the request, so the Service refuses what the window
    /// would not offer (D-184).
    #[allow(clippy::too_many_arguments)] // one call per command
    fn plan(
        &mut self,
        service: &Service,
        view: ViewId,
        caption: &str,
        op: FolderOp,
        node: NodeId,
        sides: SideSet,
        mark: Mark,
        subject: String,
    ) {
        let mut selection = Selection::default();
        selection.insert(node, sides);
        let request = FolderOpRequest {
            op: OpRequest::Op(op),
            select: SelectRequest {
                selection: Some(selection),
                display: Some(self.display),
                expand: ExpandWords::All,
                iu: self.display.ignore_unimportant,
                ..SelectRequest::default()
            },
            read_only: self.write.read_only,
            overrides: self.write.overrides(),
        };
        let model = Arc::clone(&mark.model);
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderOp,
            format!("plan: {caption}"),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                wait_at(gate.as_ref());
                let plan = svc.folder_op_plan(&model, &request, job)?;
                if job.cancel.is_cancelled() {
                    return Err(ApiError::Cancelled);
                }
                let confirmation = svc.folder_confirmation(&plan, None);
                Ok((plan, confirmation))
            },
        );
        self.task = Some(Task::Planning {
            caption: caption.to_owned(),
            view,
            mark,
            node,
            subject,
            job,
        });
    }

    /// The confirmation dialog closed: its answers, or `None` when it closed with a question open (a Cancel).
    pub fn confirmed(&mut self, service: &Service, answers: Option<Confirmation>) {
        let Some(Task::Asking {
            caption,
            plan,
            mark,
        }) = self.task.take()
        else {
            return;
        };
        let finished = answers.map(|c| c.finish(*plan));
        match finished {
            Some(Ok(plan)) => {
                if mark.holds(self.prepared()) {
                    self.execute_plan(service, &caption, plan, mark);
                } else {
                    self.refuse_changed(&caption);
                }
            }
            Some(Err(_)) | None => self.notes.push(LogNote {
                kind: LogKind::Summary,
                text: format!("{caption}: cancelled, nothing was written."),
            }),
        }
    }

    /// The plan changed under a dialog: nothing runs, and the window says so.
    fn refuse_changed(&mut self, caption: &str) {
        self.outbox.push(FolderEvent::Notice {
            kind: NoticeKind::Warning,
            text: format!(
                "{caption}: the comparison changed while the operation waited; nothing was written. Select the row again."
            ),
        });
    }

    /// The selection moved off the row while its operation was prepared: the question is not asked, nothing runs, and
    /// the window says so (the question would name a row the user no longer has selected).
    fn refuse_moved(&mut self, caption: &str) {
        self.outbox.push(FolderEvent::Notice {
            kind: NoticeKind::Warning,
            text: format!(
                "{caption}: the selection changed while the operation was prepared; nothing was written."
            ),
        });
    }

    /// Runs a confirmed plan in a job (`JobKind::FolderOp`).
    fn execute_plan(&mut self, service: &Service, caption: &str, plan: OpPlan, mark: Mark) {
        let refresh = plan.refresh.clone();
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderOp,
            caption.to_owned(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                wait_at(gate.as_ref());
                svc.folder_op_execute(plan, job)
            },
        );
        self.task = Some(Task::Executing {
            caption: caption.to_owned(),
            mark,
            refresh,
            began: Instant::now(),
            job,
        });
    }

    /// Refreshes `nodes` of the current model in a job (`folder_refresh`).
    fn refresh_rows(&mut self, service: &Service, nodes: Vec<NodeId>, after: AfterRefresh) {
        let Some(prepared) = self.prepared() else {
            return;
        };
        let model = Arc::clone(prepared.model());
        let job_model = Arc::clone(&model);
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderRefresh,
            "refresh".into(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                wait_at(gate.as_ref());
                svc.folder_refresh(&job_model, &nodes, job, &hikasa_folder::NullObserver)
            },
        );
        let points = self.restore_points();
        self.task = Some(Task::Refreshing {
            after,
            model,
            points,
            job,
        });
    }

    /// Refreshes `nodes` as after an operation (tests: a pair that changes under a child view).
    #[cfg(test)]
    pub fn refresh_for_test(&mut self, service: &Service, nodes: Vec<NodeId>) {
        self.refresh_rows(service, nodes, AfterRefresh::Operation);
    }

    /// Polls the task; true when the view changed.
    fn poll_task(&mut self, service: &Service) -> bool {
        let ready = match &self.task {
            Some(Task::Planning { job, .. }) => job.try_result().map(Ready::Plan),
            Some(Task::Executing { job, .. }) => job.try_result().map(Ready::Outcome),
            Some(Task::Refreshing { job, .. }) => job.try_result().map(Ready::Refresh),
            _ => None,
        };
        let Some(ready) = ready else {
            return false;
        };
        let Some(task) = self.task.take() else {
            return false;
        };
        match (task, ready) {
            (
                Task::Planning {
                    caption,
                    view,
                    mark,
                    node,
                    subject,
                    ..
                },
                Ready::Plan(result),
            ) => match result {
                Ok((plan, confirmation)) => {
                    if !mark.holds(self.prepared()) {
                        self.refuse_changed(&caption);
                    } else if self.selected != Some(node) {
                        self.refuse_moved(&caption);
                    } else if confirmation.next().is_none() {
                        // Nothing to ask (the options drop every question): the plan runs as confirmed.
                        match confirmation.finish(plan) {
                            Ok(plan) => self.execute_plan(service, &caption, plan, mark),
                            Err(_) => self.notes.push(LogNote {
                                kind: LogKind::Summary,
                                text: format!("{caption}: cancelled, nothing was written."),
                            }),
                        }
                    } else {
                        let dialog =
                            PlanConfirmDialog::new(view, &caption, subject, &plan, confirmation);
                        self.task = Some(Task::Asking {
                            caption,
                            plan: Box::new(plan),
                            mark,
                        });
                        self.outbox.push(FolderEvent::Dialog(Box::new(dialog)));
                    }
                }
                Err(ApiError::Cancelled) => self.notes.push(LogNote {
                    kind: LogKind::Summary,
                    text: format!("{caption}: stopped while planning, nothing was written."),
                }),
                Err(e) => self.outbox.push(FolderEvent::Notice {
                    kind: NoticeKind::Warning,
                    text: format!("{caption}: {e}"),
                }),
            },
            (
                Task::Executing {
                    caption,
                    mark,
                    refresh,
                    began,
                    ..
                },
                Ready::Outcome(result),
            ) => {
                let nodes = match result {
                    Ok(outcome) => {
                        let nodes = outcome.refresh_nodes.clone();
                        self.outbox.push(FolderEvent::Outcome {
                            caption,
                            outcome: Box::new(outcome),
                            elapsed: began.elapsed(),
                        });
                        nodes
                    }
                    Err(e) => {
                        self.outbox.push(FolderEvent::Notice {
                            kind: NoticeKind::Error,
                            text: format!("{caption}: {e}"),
                        });
                        refresh
                    }
                };
                // What was written is shown: the rows' folders, in the model the plan was made from.
                if Arc::ptr_eq(
                    &mark.model,
                    self.prepared().map_or(&mark.model, |p| p.model()),
                ) {
                    self.refresh_rows(service, nodes, AfterRefresh::Operation);
                }
            }
            (
                Task::Refreshing {
                    after,
                    model,
                    points,
                    ..
                },
                Ready::Refresh(result),
            ) => {
                let current = self
                    .prepared()
                    .is_some_and(|p| Arc::ptr_eq(p.model(), &model));
                if !current {
                    return true;
                }
                if let (Ok(dto), AfterRefresh::Archive(path)) = (&result, &after) {
                    // Opened: its row is a folder that could be listed (a flat comparison runs again with it). An
                    // archive that cannot be read is a folder that failed, with its error: not remembered.
                    let failed = dto.report.is_some().then(|| {
                        let read = model.read();
                        match live_at(&read, path, true) {
                            Some(id)
                                if read.node(id).is_some_and(|n| n.scan != ScanStatus::Failed) =>
                            {
                                None
                            }
                            Some(id) => Some(
                                read.errors(id)
                                    .first()
                                    .map_or_else(String::new, |e| e.error.message.clone()),
                            ),
                            None => Some(String::new()),
                        }
                    });
                    match failed.flatten() {
                        None => self.archive_opened(path.clone()),
                        Some(why) => self.outbox.push(FolderEvent::Notice {
                            kind: NoticeKind::Warning,
                            text: if why.is_empty() {
                                format!("Open {path}: the archive could not be opened.")
                            } else {
                                format!("Open {path}: {why}")
                            },
                        }),
                    }
                }
                match result {
                    Ok(dto) => match dto.report {
                        Some(report) => {
                            let points = points.map(|p| self.untouched(p));
                            self.renumber(&report.renumbered);
                            self.find_again(points.as_ref());
                            if let AfterRefresh::Expand(node) = after {
                                let node = renumbered(&report.renumbered, node);
                                self.set_open(node, true);
                            }
                            let read = model.read();
                            let crowded = read.garbage() > read.len() / 2;
                            drop(read);
                            if crowded {
                                // The arena holds more records a refresh left than live ones: a re-run compacts it.
                                self.refresh(service);
                                return true;
                            }
                        }
                        // A flat comparison is not refreshed: it runs again.
                        None => {
                            self.refresh(service);
                            return true;
                        }
                    },
                    Err(ApiError::Cancelled) => {}
                    Err(e) => {
                        if let AfterRefresh::Archive(path) = &after {
                            self.outbox.push(FolderEvent::Notice {
                                kind: NoticeKind::Warning,
                                text: format!("Open {path}: {e}"),
                            });
                        } else {
                            self.notes.push(LogNote {
                                kind: LogKind::Warning,
                                text: format!("refresh: {e}"),
                            });
                        }
                        if after == AfterRefresh::Operation {
                            self.refresh(service);
                            return true;
                        }
                    }
                }
                self.rebuild();
            }
            (task, _) => self.task = Some(task),
        }
        true
    }

    /// Moves the view's ids by a refresh's `renumbered`: the open folders, the selection, the child views' rows.
    fn renumber(&mut self, pairs: &[(NodeId, NodeId)]) {
        if pairs.is_empty() {
            return;
        }
        let map: HashMap<NodeId, NodeId> = pairs.iter().copied().collect();
        let follow = |id: NodeId| {
            let mut id = id;
            for _ in 0..pairs.len() {
                match map.get(&id) {
                    Some(&next) if next != id => id = next,
                    _ => break,
                }
            }
            id
        };
        self.expanded = self.expanded.iter().map(|&id| follow(id)).collect();
        self.collapsed = self.collapsed.iter().map(|&id| follow(id)).collect();
        self.selected = self.selected.map(follow);
    }

    /// After a refresh: a row whose record was replaced (new facts are a new record, not a renumbered one) is found
    /// again by path: an open folder stays open, the selected row stays selected, a child view's row follows.
    fn find_again(&mut self, points: Option<&Restore>) {
        let Some(prepared) = self.prepared() else {
            return;
        };
        let model = Arc::clone(prepared.model());
        let read = model.read();
        let at = |path: &str, folder: bool| live_at(&read, path, folder);
        if let Some(points) = points {
            let found: Vec<NodeId> = points
                .folders
                .iter()
                .filter_map(|(p, _)| at(p, true))
                .collect();
            if self.expand_all {
                self.collapsed.extend(found);
            } else {
                self.expanded.extend(found);
            }
            if let Some((path, folder)) = &points.selected
                && let Some(live) = at(path, *folder)
            {
                self.selected = Some(live);
            }
        }
    }

    // ---- FC-029(c): the result of a child view -------------------------------------------------------------------

    /// A child view was opened for `ticket`'s row.
    pub fn adopt_child(&mut self, child: ViewId, ticket: ChildTicket) {
        self.children.insert(child, ticket);
    }

    /// True when `child` is a child view of this one whose result is not in yet.
    #[cfg(test)]
    pub fn has_child(&self, child: ViewId) -> bool {
        self.children.contains_key(&child)
    }

    /// The comparison of a child view ended: its result goes to its row, now or when the view's task has ended,
    /// while the pair still has the stamps it had at `Open`; a pair changed meanwhile keeps its old result.
    pub fn child_result(&mut self, child: ViewId, outcome: ContentOutcome) {
        if !self.children.contains_key(&child) {
            return;
        }
        self.child_results.push((child, outcome));
        if self.task.is_none() {
            self.apply_child_results();
        }
    }

    fn apply_child_results(&mut self) {
        let Some(model) = self.prepared().map(|p| Arc::clone(p.model())) else {
            self.child_results.clear();
            return;
        };
        let mut applied = false;
        for (child, outcome) in std::mem::take(&mut self.child_results) {
            if let Some(ticket) = self.children.remove(&child) {
                // The live file row at the ticket's path (a refresh may have replaced its record); none, nothing.
                let row = {
                    let read = model.read();
                    file_row_at(&read, &ticket.path)
                };
                if let Some(row) = row {
                    applied |= model.apply_child_result(row, ticket.stamps, outcome);
                }
            }
        }
        if applied {
            self.rebuild();
        }
    }

    /// Opens the row of the archive `Open` added, and the folders around it, once it is in the tree.
    fn open_pending_archive(&mut self) {
        let Some(path) = self.expand_archive.as_deref() else {
            return;
        };
        let Some(prepared) = self.prepared() else {
            return;
        };
        let model = Arc::clone(prepared.model());
        let read = model.read();
        let found = read.preorder().map(|(id, _)| id).find(|&id| {
            read.path(id).as_str() == path && read.node(id).is_some_and(|n| n.is_folder())
        });
        if let Some(id) = found {
            for folder in read.ancestors(id).into_iter().chain([id]) {
                self.set_open(folder, true);
            }
            self.expand_archive = None;
        }
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
        if let Some(prepared) = self.prepared() {
            request
                .format_overrides
                .clone_from(prepared.format_overrides());
        }
        Some((
            request,
            ViewOptions {
                filter: TextDisplayFilter::All,
                ignore_unimportant: self.display.ignore_unimportant,
                expanded: Vec::new(),
            },
        ))
    }

    /// `Open` on a file pair (FC-062), with the row and its stamps for the child's result (FC-029(c)).
    fn open_pair(&self) -> Option<OpenPair> {
        let (request, view) = self.open_request()?;
        let ticket = self.selected.and_then(|id| {
            let prepared = self.prepared()?;
            let read = prepared.model().read();
            let node = read.node(id)?;
            Some(ChildTicket {
                path: read.path(id),
                stamps: Pair::new(stamp_of(node.left.as_ref()), stamp_of(node.right.as_ref())),
            })
        });
        Some(OpenPair {
            request,
            view,
            ticket,
        })
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
        ctx.writable = [
            self.write.writable(Side::Left),
            self.write.writable(Side::Right),
        ];
        if let Some((_, on)) = self.operable_row() {
            ctx.row_selected = true;
            ctx.row_on = on;
        }
        if let Some((_, open)) = self.selected_folder() {
            ctx.open_folder_selected = open;
            ctx.closed_folder_selected = !open;
        }
        ctx.pair_selected =
            self.selected_pair().is_some() || self.selected_closed_archive().is_some();
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
    pub fn execute(&mut self, service: &Service, id: CommandId) -> Option<Option<OpenPair>> {
        match id {
            CommandId::Stop => self.cancel(),
            // An operation under way ends first: its outcome is reported and its rows refreshed.
            CommandId::FullRefresh => {
                if self.task.is_none() {
                    self.refresh(service);
                }
            }
            CommandId::Expand => {
                if let Some((node, _)) = self.selected_folder() {
                    self.expand(service, node);
                }
            }
            CommandId::Collapse => {
                if let Some((node, _)) = self.selected_folder() {
                    self.set_open(node, false);
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
            CommandId::Open => {
                if let Some((node, path)) = self.selected_closed_archive() {
                    self.open_archive(service, node, path);
                    return Some(None);
                }
                return Some(self.open_pair());
            }
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
                self.scanning(),
            ),
        }
    }

    /// The log panel is shown.
    pub fn show_log(&self) -> bool {
        self.show_log
    }

    /// Shows the log panel (a notice's *Show log*).
    pub fn show_log_panel(&mut self) {
        self.show_log = true;
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
        if let Some((title, phase, fraction)) = self.task.as_ref().and_then(Task::job_status) {
            return vec![JobStatus {
                view,
                title,
                phase,
                fraction,
            }];
        }
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
            status.state = if self.scanning() {
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
        status.state = if self.scanning() {
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

/// The result of a child Text Compare view for its row (FC-029(c)): the child's own, rules-based, with unimportant
/// differences kept as they are (the folder view's display decides how they count).
pub fn child_outcome(result: ContentResult) -> ContentOutcome {
    ContentOutcome {
        method: ContentMethod::Rules,
        result,
        warnings: Vec::new(),
    }
}

/// A job's result, by task.
enum Ready {
    Plan(Result<(OpPlan, Confirmation), ApiError>),
    Outcome(Result<FolderOpOutcome, ApiError>),
    Refresh(Result<RefreshDto, ApiError>),
}

/// `id` after `renumbered`.
fn renumbered(pairs: &[(NodeId, NodeId)], id: NodeId) -> NodeId {
    let mut id = id;
    for _ in 0..pairs.len() {
        match pairs.iter().find(|(old, _)| *old == id) {
            Some(&(_, new)) if new != id => id = new,
            _ => break,
        }
    }
    id
}

/// The live row at `path`, a folder or not (a file and a folder of one name are two rows, D-20); `node_at` walks the
/// tree, so a record a refresh left behind is never found.
fn live_at(read: &TreeRead<'_>, path: &str, folder: bool) -> Option<NodeId> {
    let rel = RelPath::parse(path).ok()?;
    let first = read.node_at(&rel)?;
    let parent = read.node(first)?.parent.unwrap_or(NodeId::ROOT);
    std::iter::once(first)
        .chain(read.children(parent))
        .find(|&c| {
            read.path(c).as_str() == path && read.node(c).is_some_and(|n| n.is_folder() == folder)
        })
}

/// The file row at `path` (a file and a folder of one name are two rows, D-20).
fn file_row_at(read: &TreeRead<'_>, path: &RelPath) -> Option<NodeId> {
    let first = read.node_at(path)?;
    let parent = read.node(first)?.parent.unwrap_or(NodeId::ROOT);
    std::iter::once(first)
        .chain(read.children(parent))
        .find(|&id| read.path(id) == *path && read.node(id).is_some_and(|n| !n.is_folder()))
}

/// The word of an entry kind in the Delete dialog.
fn kind_word(kind: hikasa_vfs::EntryKind) -> &'static str {
    match kind {
        hikasa_vfs::EntryKind::File => "file",
        hikasa_vfs::EntryKind::Dir => "folder",
        hikasa_vfs::EntryKind::Symlink => "link",
        hikasa_vfs::EntryKind::Other => "special file",
    }
}

/// The stamp of one side of a row: what `apply_child_result` compares.
fn stamp_of(entry: Option<&EntryView<'_>>) -> ItemStamp {
    match entry {
        Some(e) => ItemStamp {
            kind: e.kind,
            size: Some(e.size),
            mtime: e.mtime,
        },
        None => ItemStamp {
            kind: hikasa_vfs::EntryKind::File,
            size: None,
            mtime: None,
        },
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

/// True when `Open` on this row opens a closed archive: the flag and the rules (D-153).
fn opens_archive(read: &TreeRead<'_>, node: &NodeView<'_>) -> bool {
    node.flags.contains(NodeFlags::ARCHIVE)
        && !node.is_folder()
        && read.rules().handling.archives == ArchiveHandling::OnceOpened
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
    let archive = node.flags.contains(NodeFlags::ARCHIVE);
    let opens_archive = opens_archive(read, &node);
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
        archive,
        opens_archive,
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
