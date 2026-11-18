//! The scripted performance mode of ARCH-GUI-003 (`HIKASA_GUI_BENCH=scroll|folder|two_windows|workspace`, see
//! `main`; `two_windows` is the M2 W4a gate of §1.4.17: the text scroll with a second window holding Home;
//! `workspace` is G12: the first screen of a restored workspace, its tabs pending; `options` is the memory of the
//! Options dialog, every page shown once): the window
//! opens the command line's view and drives it frame by frame — scrolling the text panes through the whole
//! comparison, or scrolling the folder tree while the scan runs — records the first-screen marks, the frame
//! times (the UI thread's work per frame, the frame intervals and eframe's CPU time per frame) and the memory
//! of the process (PSS and RSS), prints one JSON summary on stdout and quits.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hikasa_session::ipc::ViewId;
use serde_json::{Value, json};

use crate::app::{AppCore, ViewModel};
use crate::ui::folder::FolderUi;
use crate::ui::table::TableUi;
use crate::ui::text::TextUi;

/// What a scroll phase moves: the text panes' or the grid's first row (Table Compare phase 3).
trait Scrolled {
    fn top_mut(&mut self) -> &mut f64;
    fn page(&self) -> f64;
    fn max(&self) -> f64;
}

impl Scrolled for TextUi {
    fn top_mut(&mut self) -> &mut f64 {
        &mut self.top
    }
    fn page(&self) -> f64 {
        self.page_rows
    }
    fn max(&self) -> f64 {
        self.max_top()
    }
}

impl Scrolled for TableUi {
    fn top_mut(&mut self) -> &mut f64 {
        &mut self.top
    }
    fn page(&self) -> f64 {
        self.page_rows
    }
    fn max(&self) -> f64 {
        self.max_top()
    }
}

/// Which benchmark runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchKind {
    /// Text Compare: first screen and scrolling (ARCH-GUI-003 (a)).
    Scroll,
    /// Folder Compare: incremental rendering while scanning (acceptance (b)).
    Folder,
    /// The text scroll with a second window (Home) open (M2 W4a, D-132).
    TwoWindows,
    /// A restored workspace: the first screen of its active tab (M2 W4b, G12).
    Workspace,
    /// The Options dialog: every page once, then idle frames (M2 W4c, D-132).
    Options,
    /// G15 tree (M2 W9a, D-257): `Folder` with every column shown and the sort Size descending on the left.
    FolderColumns,
    /// G15 select: `Folder`'s tree, then Select All Files (an extra figure), Select All, Invert Selection, Shift+Left and
    /// 300 frames of Shift+Down.
    Select,
    /// G15 completion: 10 characters typed in the left path edit, the folder holding 10,000 entries.
    Completion,
    /// G15 browse: the browse dialog on the left base (100,000 entries), then 600 frames of scrolling.
    Browse,
    /// G15 preview (M2 W9b): `Folder`'s tree, then Select All on the left and Copy to Right; the preview's first frame,
    /// then 600 frames of scrolling its list.
    Preview,
    /// G15 sync (M2 W9b): a Folder Sync view of the tree with Mirror to Right: the plan after the comparison, an
    /// override of one row, then 600 frames of scrolling the tree.
    SyncView,
    /// The Table Compare gate (W13, m2-table-contract §6.6): the text scroll's three phases on the grid of a table
    /// pair (`table_scroll`).
    TableScroll,
    /// G16 type, changed, lines, burst (M2 W11a, D-322): `text_typing`.
    TextTyping,
    /// G16 undo: `text_undo`.
    TextUndo,
    /// ARCH-GUI-003 edited: `text_edited`.
    TextEdited,
    /// G16 wrap (M2 W11c): `text_wrap`.
    TextWrap,
    /// G16 find: `text_find`.
    TextFind,
    /// G16 find files: `find_files`.
    FindFiles,
    /// G16 save: `text_save`.
    TextSave,
    /// G10 in the window (M2 W13a, D-372): `hex_open`, `hex_scroll`, `hex_align`, `hex_find`.
    Hex(crate::bench_views::HexBenchKind),
    /// G11 in the window (M2 W13b, D-372): `picture_open`, `picture_zoom`, `picture_pan`, `picture_drag`,
    /// `picture_tolerance` (`bench_picture.rs`).
    Picture(crate::bench_picture::PictureMode),
}

impl BenchKind {
    /// The editing benchmark of this kind (M2 W11a).
    fn edit_mode(self) -> Option<crate::bench_text::EditMode> {
        match self {
            BenchKind::TextTyping => Some(crate::bench_text::EditMode::Typing),
            BenchKind::TextUndo => Some(crate::bench_text::EditMode::Undo),
            BenchKind::TextEdited => Some(crate::bench_text::EditMode::Edited),
            BenchKind::TextWrap => Some(crate::bench_text::EditMode::Wrap),
            BenchKind::TextFind => Some(crate::bench_text::EditMode::Find),
            BenchKind::FindFiles => Some(crate::bench_text::EditMode::FindFiles),
            BenchKind::TextSave => Some(crate::bench_text::EditMode::Save),
            _ => None,
        }
    }
}

impl BenchKind {
    /// The kinds that scan `Folder`'s tree while the tree scrolls.
    fn scans(self) -> bool {
        matches!(
            self,
            BenchKind::Folder | BenchKind::FolderColumns | BenchKind::Select
        )
    }
}

/// `preview`, `sync_view`: frames of scrolling.
const W9B_SCROLL_FRAMES: u32 = 600;

/// `select`: frames of Shift+Down after the three commands.
const SELECT_DOWN_FRAMES: u32 = 300;
/// `select`, `completion`, `browse`: idle frames before the script starts and after it ends.
const W9A_IDLE_FRAMES: u32 = 30;
/// `completion`: the characters typed, one a frame.
const TYPED: &str = "file_01234";
/// `browse`: frames of scrolling the listing.
const BROWSE_SCROLL_FRAMES: u32 = 600;
/// `browse`: points scrolled a frame (about three rows).
const BROWSE_SCROLL_STEP: f32 = 54.0;

/// One scroll phase of the text benchmark.
#[derive(Debug, Clone, Copy)]
struct PhaseSpec {
    name: &'static str,
    frames: u32,
    step: Step,
    wrap: bool,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// A fixed number of rows per frame (a mouse wheel).
    Rows(f64),
    /// A page per frame.
    Page,
    /// The whole comparison in the phase's frames.
    Sweep,
}

const PHASES: &[PhaseSpec] = &[
    PhaseSpec {
        name: "wheel_3_lines_per_frame",
        frames: 600,
        step: Step::Rows(3.0),
        wrap: true,
    },
    PhaseSpec {
        name: "page_per_frame",
        frames: 300,
        step: Step::Page,
        wrap: true,
    },
    PhaseSpec {
        name: "sweep_whole_comparison",
        frames: 1200,
        step: Step::Sweep,
        wrap: false,
    },
];

/// Frames to let pass after the coloured screen before the first phase.
const WARM_UP_FRAMES: u32 = 30;
/// The same for `two_windows`, whose second window opens at the first screen.
const TWO_WINDOWS_WARM_UP_FRAMES: u32 = 240;
/// `workspace`: idle frames drawn after the coloured screen with no job left, before the memory is sampled.
const WORKSPACE_TAIL_FRAMES: u32 = 60;
/// `scroll`, `two_windows`: idle frames after the last phase, before the memory is sampled (D-132).
const SCROLL_TAIL_FRAMES: u32 = 60;
/// `options`: frames before the dialog opens, and on each page.
const OPTIONS_FRAMES_PER_PAGE: u32 = 2;
/// `options`: idle frames after the last page, before the memory is sampled.
const OPTIONS_TAIL_FRAMES: u32 = 60;
/// Folder benchmark: rows the tree scrolls per frame.
const FOLDER_ROWS_PER_FRAME: f64 = 2.0;
/// Folder benchmark: how long frames keep being drawn after the scan.
const FOLDER_TAIL: Duration = Duration::from_millis(500);

/// Samples of one series.
#[derive(Debug, Default)]
struct Samples {
    work: Vec<f64>,
    interval: Vec<f64>,
    cpu: Vec<f64>,
    slices: Vec<f64>,
}

impl Samples {
    fn json(&self) -> Value {
        json!({
            "frame_work_ms": stats(&self.work),
            "frame_interval_ms": stats(&self.interval),
            "eframe_cpu_ms": stats(&self.cpu),
            "slice_build_ms": stats(&self.slices),
        })
    }
}

#[derive(Debug)]
enum Script {
    /// Waiting for the view (text: its coloured screen) plus the warm-up frames.
    Waiting(u32),
    /// A text scroll phase runs.
    Phase {
        index: usize,
        frame: u32,
        started: Instant,
        step: f64,
        samples: Samples,
    },
    /// Idle frames after the last phase.
    Tail(u32),
    /// The folder tree scrolls while the scan runs.
    Folder,
    /// Finished: report and quit.
    Done,
}

/// Benchmark state (UI thread).
#[derive(Debug)]
pub struct BenchState {
    kind: BenchKind,
    process_start: Instant,
    view: Option<ViewId>,
    view_opened: Option<Instant>,
    first_frame: Option<Instant>,
    frame_start: Option<Instant>,
    last_frame_start: Option<Instant>,
    drawn_pending: [Option<()>; 2],
    first_screen: Option<Instant>,
    ready_screen: Option<Instant>,
    script: Script,
    phases: Vec<Value>,
    all: Samples,
    scanning: Samples,
    scan_done: Option<Instant>,
    renderer: Value,
    window: Value,
    reported: bool,
    frames: u64,
    first_frame_work: Option<f64>,
    /// The memory when the first screen came up.
    first_screen_memory: Value,
    /// `workspace`: the memory after the coloured screen, every job done and a few idle frames (G12 compares it:
    /// at the first screen the comparison is still running, so a sample there measures its progress).
    settled_memory: Value,
    /// `workspace`: idle frames since the coloured screen.
    tail: u32,
    /// `two_windows`: the second window was asked for.
    second_window: bool,
    /// `two_windows`: the second window opens at the coloured screen instead of the first screen
    /// (`HIKASA_PERF_SECOND_WINDOW=coloured`; the late variant of the gate).
    second_after_coloured: bool,
    /// `options`: the frames of the script.
    options_frame: u32,
    /// W9a scripts (D-257): frames since their start, the label of this frame's action and the labelled works.
    w9a_frame: u32,
    w9a_label: Option<&'static str>,
    w9a_works: Vec<(&'static str, f64)>,
    /// `completion`: when the last key was typed and when the list showed; `browse`: when the dialog opened and was
    /// listed.
    w9a_mark: Option<Instant>,
    w9a_done_ms: Option<f64>,
    /// `sync_view`: the plan's time after the comparison ended, the override's re-plan, and the tree scrolls.
    w9b_plan_ms: Option<f64>,
    /// `preview`: the plan job's own time (`FolderViewModel::plan_job_time`).
    w9b_plan_job_ms: Option<f64>,
    w9b_override_ms: Option<f64>,
    w9b_scrolling: bool,
    /// `preview`: the lines of the preview list (the items and the summary).
    w9b_lines: Option<usize>,
    /// G16's editing script (M2 W11a).
    edit: Option<crate::bench_text::EditBench>,
    /// The last frame's work (ms), for the editing script's key frames.
    last_work: Option<f64>,
    /// The hex view's script (M2 W13a).
    hex: Option<crate::bench_views::HexBench>,
    /// G11's picture script (M2 W13b).
    picture: Option<crate::bench_picture::PictureBench>,
}

impl BenchState {
    /// A benchmark started at `process_start`.
    pub fn new(kind: BenchKind, process_start: Instant) -> BenchState {
        BenchState {
            kind,
            process_start,
            view: None,
            view_opened: None,
            first_frame: None,
            frame_start: None,
            last_frame_start: None,
            drawn_pending: [None, None],
            first_screen: None,
            ready_screen: None,
            script: Script::Waiting(0),
            phases: Vec::new(),
            all: Samples::default(),
            scanning: Samples::default(),
            scan_done: None,
            renderer: Value::Null,
            window: Value::Null,
            reported: false,
            frames: 0,
            first_frame_work: None,
            first_screen_memory: Value::Null,
            settled_memory: Value::Null,
            tail: 0,
            second_window: false,
            second_after_coloured: std::env::var_os("HIKASA_PERF_SECOND_WINDOW")
                .is_some_and(|v| v == "coloured"),
            options_frame: 0,
            w9a_frame: 0,
            w9a_label: None,
            w9a_works: Vec::new(),
            w9a_mark: None,
            w9a_done_ms: None,
            w9b_plan_ms: None,
            w9b_plan_job_ms: None,
            w9b_override_ms: None,
            w9b_scrolling: false,
            w9b_lines: None,
            edit: kind.edit_mode().map(crate::bench_text::EditBench::new),
            last_work: None,
            hex: match kind {
                BenchKind::Hex(k) => Some(crate::bench_views::HexBench::new(k)),
                _ => None,
            },
            picture: match kind {
                BenchKind::Picture(mode) => {
                    Some(crate::bench_picture::PictureBench::new(mode, process_start))
                }
                _ => None,
            },
        }
    }

    /// True for a G11 picture script (its window is 1,600 × 1,000 points).
    pub fn picture(&self) -> bool {
        self.picture.is_some()
    }

    /// G11's picture script (M2 W13b): this frame's step, before the view is drawn.
    pub fn drive_picture(&mut self, core: &mut AppCore) {
        let view = self.view;
        if let Some(p) = self.picture.as_mut() {
            p.drive(core, view);
        }
    }

    /// G11's picture script: the frame's work (after [`BenchState::end_frame`]).
    pub fn end_frame_picture(&mut self, core: &mut AppCore) {
        let view = self.view;
        if let Some(p) = self.picture.as_mut() {
            p.end_frame(core, view, self.last_work);
        }
    }

    /// The hex view's script (M2 W13a), on the window's core before the frame is drawn.
    pub fn drive_hex(&mut self, core: &mut AppCore) {
        if let Some(h) = self.hex.as_mut() {
            h.drive(core);
        }
    }

    /// The hex view's frame end: its work and the screen's state.
    pub fn end_frame_hex(&mut self, hexes: &HashMap<ViewId, crate::ui::hex::HexUi>) {
        let (Some(h), Some(work)) = (self.hex.as_mut(), self.last_work) else {
            return;
        };
        h.end_frame(work, hexes);
    }

    /// The W9b G15 scripts (D-257): `preview` and `sync_view`, run on the window's core before the frame is drawn.
    pub fn drive_w9b(&mut self, core: &mut AppCore) {
        use crate::commands::CommandId;
        if !matches!(self.kind, BenchKind::Preview | BenchKind::SyncView) {
            return;
        }
        let Some(view) = self.view else {
            return;
        };
        let root = core.root();
        if self.kind == BenchKind::SyncView && self.w9a_frame == 0 {
            // Mirror to Right before the comparison ends: its first plan is the one measured.
            core.sync_preset(view, hikasa_folder::sync::SyncPreset::MirrorRight);
        }
        let busy = core
            .tab(view)
            .is_some_and(|t| matches!(t.model(), Some(ViewModel::Folder(m)) if m.busy()));
        if busy && self.scan_done.is_none() {
            self.w9a_frame = 1;
            return;
        }
        if self.scan_done.is_none() {
            self.scan_done = Some(Instant::now());
        }
        self.w9a_frame += 1;
        match self.kind {
            BenchKind::Preview => {
                let k = self.w9a_frame.saturating_sub(W9A_IDLE_FRAMES);
                if self.w9a_frame == 2 {
                    // Every row shown (the default display shows the differences only): the copy takes every file.
                    core.execute(root, CommandId::FavCompareAll);
                }
                if self.w9a_frame <= W9A_IDLE_FRAMES {
                    return;
                }
                if k == 1 {
                    // The view's memory before the preview opens (PSS while open ≤ the view's + 96 MiB).
                    self.settled_memory = memory();
                    if let Some(m) = folder_of(core, view) {
                        m.select_all();
                        m.restrict(hikasa_core::Side::Left);
                    }
                    self.w9a_label = Some("command");
                    core.execute(root, CommandId::CopyRight);
                    self.w9a_mark = Some(Instant::now());
                    return;
                }
                let Some(d) = core
                    .window_mut(root)
                    .and_then(|w| w.dialogs.find_mut::<crate::op_dialogs::PlanConfirmDialog>())
                else {
                    self.w9a_label = Some("planning");
                    if k > 60_000 {
                        self.script = Script::Done;
                    }
                    return;
                };
                if self.w9a_done_ms.is_none() {
                    self.w9b_lines = d.list().map(crate::op_preview::PreviewList::len);
                    self.w9b_plan_job_ms = folder_of(core, view)
                        .and_then(|m| m.plan_job_time)
                        .map(|t| t.as_secs_f64() * 1000.0);
                    self.w9a_done_ms = self.w9a_mark.map(|at| at.elapsed().as_secs_f64() * 1000.0);
                    self.w9a_label = Some("first_preview_frame");
                    self.tail = 0;
                    return;
                }
                self.tail += 1;
                if self.tail <= W9B_SCROLL_FRAMES {
                    self.w9a_label = Some("scrolling");
                    d.bench_scroll = Some(self.tail as f32 * BROWSE_SCROLL_STEP);
                } else if self.tail > W9B_SCROLL_FRAMES + W9A_IDLE_FRAMES {
                    self.script = Script::Done;
                }
            }
            BenchKind::SyncView => {
                let planned = match core.tab(view).and_then(|t| t.model()) {
                    Some(ViewModel::Folder(m)) => m
                        .sync()
                        .is_some_and(|s| s.plan().is_some() && !s.planning()),
                    _ => false,
                };
                if self.w9b_plan_ms.is_none() {
                    if planned {
                        self.w9b_plan_ms =
                            self.scan_done.map(|at| at.elapsed().as_secs_f64() * 1000.0);
                    } else {
                        self.w9a_label = Some("planning");
                    }
                    return;
                }
                if self.w9a_mark.is_none() {
                    // An override of one row: a row the plan copies when one is shown, else the first row.
                    if let Some(m) = folder_of(core, view) {
                        let row = m
                            .window()
                            .rows
                            .iter()
                            .find(|r| {
                                r.sync.is_some_and(|a| {
                                    a.action == hikasa_folder::sync::SyncAction::CopyRight
                                })
                            })
                            .map_or(0, |r| r.index as usize);
                        m.select(row);
                    }
                    self.w9a_label = Some("override");
                    core.execute(root, CommandId::SyncLeave);
                    self.w9a_mark = Some(Instant::now());
                    return;
                }
                if self.w9b_override_ms.is_none() {
                    if planned {
                        self.w9b_override_ms =
                            self.w9a_mark.map(|at| at.elapsed().as_secs_f64() * 1000.0);
                        self.tail = 0;
                    } else {
                        self.w9a_label = Some("replanning");
                    }
                    return;
                }
                self.tail += 1;
                if self.tail <= W9B_SCROLL_FRAMES {
                    self.w9a_label = Some("scrolling");
                    self.w9b_scrolling = true;
                } else {
                    self.w9b_scrolling = false;
                    if self.tail > W9B_SCROLL_FRAMES + W9A_IDLE_FRAMES {
                        self.script = Script::Done;
                    }
                }
            }
            _ => {}
        }
    }

    /// The W9a G15 scripts (D-257), run on the window's core before the frame is drawn.
    pub fn drive_w9a(&mut self, core: &mut AppCore) {
        use crate::commands::CommandId;
        if !matches!(
            self.kind,
            BenchKind::FolderColumns
                | BenchKind::Select
                | BenchKind::Completion
                | BenchKind::Browse
        ) {
            return;
        }
        let Some(view) = self.view else {
            return;
        };
        let root = core.root();
        if self.kind == BenchKind::FolderColumns {
            if self.w9a_frame == 0
                && let Some(ViewModel::Folder(m)) = core.tab_mut(view).and_then(|t| t.model_mut())
            {
                m.change_columns(|c| {
                    for id in crate::folder::columns::ColumnId::ALL {
                        if !c.is_visible(id) && id != crate::folder::columns::ColumnId::Path {
                            c.toggle(id);
                        }
                    }
                    c.sort = Some(hikasa_folder::view::SortKey {
                        column: crate::folder::columns::ColumnId::Size,
                        side: hikasa_core::Side::Left,
                        descending: true,
                    });
                });
            }
            self.w9a_frame += 1;
            return;
        }
        // The other scripts start once the comparison has ended and the window has been idle a little.
        let busy = core
            .tab(view)
            .is_some_and(|t| matches!(t.model(), Some(ViewModel::Folder(m)) if m.busy()));
        if busy && self.w9a_frame == 0 {
            return;
        }
        if self.scan_done.is_none() {
            self.scan_done = Some(Instant::now());
        }
        self.w9a_frame += 1;
        let k = self.w9a_frame.saturating_sub(W9A_IDLE_FRAMES);
        if self.w9a_frame <= W9A_IDLE_FRAMES {
            return;
        }
        match self.kind {
            BenchKind::Select => match k {
                // Select All Files first, an extra figure; then the contract's script: Select All (every shown row,
                // every present side), Invert Selection, Shift+Left, Shift+Down.
                1 => {
                    self.w9a_label = Some("select_all_files");
                    core.execute(root, CommandId::SelectAllFiles);
                }
                2 => {
                    self.w9a_label = Some("select_all");
                    if let Some(m) = folder_of(core, view) {
                        m.select_all();
                    }
                }
                3 => {
                    self.w9a_label = Some("invert_selection");
                    core.execute(root, CommandId::SelectInvert);
                }
                4 => {
                    self.w9a_label = Some("shift_left");
                    if let Some(m) = folder_of(core, view) {
                        m.restrict(hikasa_core::Side::Left);
                    }
                }
                k if k < 5 + SELECT_DOWN_FRAMES => {
                    self.w9a_label = Some("shift_down");
                    if let Some(m) = folder_of(core, view) {
                        let shift = crate::folder::selection::Mods {
                            shift: true,
                            ..Default::default()
                        };
                        m.move_by(1, shift);
                    }
                }
                k if k > 5 + SELECT_DOWN_FRAMES + W9A_IDLE_FRAMES => self.script = Script::Done,
                _ => {}
            },
            BenchKind::Completion => {
                let typed = TYPED.chars().count() as u32;
                if k == 1
                    && let Some(m) = folder_of(core, view)
                {
                    let base = m.path_edit(0).committed.clone();
                    m.path_edit(0)
                        .set_text(&format!("{base}{}", std::path::MAIN_SEPARATOR));
                }
                if (2..2 + typed).contains(&k)
                    && let Some(m) = folder_of(core, view)
                {
                    self.w9a_label = Some("typing");
                    let c = TYPED.chars().nth((k - 2) as usize).unwrap_or('x');
                    m.path_edit(0).text.push(c);
                    m.path_edit(0).edited(Instant::now());
                    if k == 1 + typed {
                        self.w9a_mark = Some(Instant::now());
                    }
                }
                if k > 1 + typed && self.w9a_done_ms.is_none() {
                    self.w9a_label = Some("waiting");
                    if let (Some(m), Some(at)) = (folder_of(core, view), self.w9a_mark)
                        && m.path_edit(0).open
                    {
                        self.w9a_done_ms = Some(at.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                if self.w9a_done_ms.is_some() {
                    self.w9a_label = None;
                    self.tail += 1;
                    if self.tail > W9A_IDLE_FRAMES {
                        self.script = Script::Done;
                    }
                }
                if k > 1 + typed + 2_000 {
                    self.script = Script::Done;
                }
            }
            BenchKind::Browse => {
                if k == 1 {
                    let start = folder_of(core, view)
                        .map(|m| m.path_edit(0).committed.clone())
                        .unwrap_or_default();
                    core.open_browse(
                        root,
                        crate::browse::BrowseMode::Folder,
                        start,
                        crate::browse::BrowseTarget::PathEdit { view, side: 0 },
                    );
                    self.w9a_mark = Some(Instant::now());
                    return;
                }
                let Some(d) = core
                    .window_mut(root)
                    .and_then(|w| w.dialogs.find_mut::<crate::browse::BrowseDialog>())
                else {
                    self.script = Script::Done;
                    return;
                };
                if self.w9a_done_ms.is_none() {
                    if !d.listing()
                        && !d.entries.is_empty()
                        && let Some(at) = self.w9a_mark
                    {
                        self.w9a_done_ms = Some(at.elapsed().as_secs_f64() * 1000.0);
                        self.tail = 0;
                    }
                    if k > 20_000 {
                        self.script = Script::Done;
                    }
                    return;
                }
                self.tail += 1;
                if self.tail <= BROWSE_SCROLL_FRAMES {
                    self.w9a_label = Some("scrolling");
                    d.bench_scroll = Some(self.tail as f32 * BROWSE_SCROLL_STEP);
                } else {
                    self.w9a_label = None;
                    if self.tail > BROWSE_SCROLL_FRAMES + W9A_IDLE_FRAMES {
                        self.settled_memory = memory();
                        self.script = Script::Done;
                    }
                }
            }
            _ => {}
        }
    }

    /// `options` (D-132): runs the script on the window's core, before the frame is drawn: `ProgramOptions`, every
    /// page for a few frames, then idle frames; the memory is sampled at the end.
    pub fn drive_options(&mut self, core: &mut AppCore) {
        if self.kind != BenchKind::Options {
            return;
        }
        let window = core.root();
        let frame = self.options_frame;
        self.options_frame += 1;
        if frame == OPTIONS_FRAMES_PER_PAGE {
            core.execute(window, crate::commands::CommandId::ProgramOptions);
            self.first_screen = Some(Instant::now());
            self.first_screen_memory = memory();
        }
        let pages = crate::options::pages::PAGES;
        let total = OPTIONS_FRAMES_PER_PAGE * (pages.len() as u32 + 1);
        if frame > OPTIONS_FRAMES_PER_PAGE && frame < total {
            let k = (frame / OPTIONS_FRAMES_PER_PAGE - 1) as usize;
            if let Some(d) = core
                .window_mut(window)
                .and_then(|w| w.dialogs.find_mut::<crate::options::OptionsDialog>())
            {
                d.show_page(pages[k.min(pages.len() - 1)].id);
            }
        }
        if frame >= total + OPTIONS_TAIL_FRAMES {
            self.settled_memory = memory();
            self.script = Script::Done;
        }
    }

    /// The view under test, opened now.
    pub fn set_view(&mut self, view: ViewId) {
        if let Some(e) = self.edit.as_mut() {
            e.set_view(view);
        }
        self.view = Some(view);
        self.view_opened = Some(Instant::now());
    }

    /// `two_windows`: true once, when the first screen is up (or the coloured screen with
    /// `HIKASA_PERF_SECOND_WINDOW=coloured`) — the window then runs `WindowNew`.
    pub fn take_second_window(&mut self) -> bool {
        let due = if self.second_after_coloured {
            self.ready_screen.is_some()
        } else {
            self.first_screen.is_some()
        };
        if self.kind == BenchKind::TwoWindows && !self.second_window && due {
            self.second_window = true;
            true
        } else {
            false
        }
    }

    /// Records the renderer (adapter) the window runs on.
    pub fn set_renderer(&mut self, renderer: Value) {
        self.renderer = renderer;
    }

    /// A frame starts. A screen drawn in the previous frame has been presented now.
    pub fn begin_frame(&mut self, cpu_of_last_frame: Option<f32>) {
        let now = Instant::now();
        self.first_frame.get_or_insert(now);
        if self.drawn_pending[0].take().is_some() && self.first_screen.is_none() {
            self.first_screen = Some(now);
            self.first_screen_memory = memory();
        }
        if self.drawn_pending[1].take().is_some() && self.ready_screen.is_none() {
            self.ready_screen = Some(now);
        }
        self.last_frame_start = self.frame_start.replace(now);
        if let (Some(cpu), true) = (cpu_of_last_frame, self.frames >= 2) {
            let ms = f64::from(cpu) * 1000.0;
            if let Script::Phase { samples, .. } = &mut self.script {
                samples.cpu.push(ms);
            }
            push(&mut self.all.cpu, ms);
            if self.kind.scans() && self.scan_done.is_none() {
                push(&mut self.scanning.cpu, ms);
            }
        }
    }

    /// Moves the view under test before it is drawn (the script's step for this frame).
    pub fn drive(
        &mut self,
        core: &AppCore,
        texts: &mut HashMap<ViewId, TextUi>,
        tables: &mut HashMap<ViewId, TableUi>,
        folders: &mut HashMap<ViewId, FolderUi>,
    ) {
        if self.kind == BenchKind::Workspace && self.view.is_none() {
            // The restored window's active tab, once it is a live view.
            if let Some(tab) = core.active_tab(core.root())
                && matches!(tab.model(), Some(ViewModel::Text(_)))
            {
                self.set_view(tab.id);
            }
        }
        let Some(view) = self.view else {
            return;
        };
        match self.kind {
            BenchKind::Scroll | BenchKind::TwoWindows => {
                self.drive_text(texts.get_mut(&view).map(|s| s as &mut dyn Scrolled));
            }
            BenchKind::TableScroll => {
                self.drive_text(tables.get_mut(&view).map(|s| s as &mut dyn Scrolled));
            }
            BenchKind::Workspace => {
                if self.ready_screen.is_some() && !core.busy() {
                    self.tail += 1;
                    if self.tail >= WORKSPACE_TAIL_FRAMES {
                        self.settled_memory = memory();
                        self.script = Script::Done;
                    }
                }
            }
            BenchKind::Options | BenchKind::Hex(_) => {}
            BenchKind::TextTyping
            | BenchKind::TextUndo
            | BenchKind::TextEdited
            | BenchKind::TextWrap
            | BenchKind::TextFind
            | BenchKind::FindFiles
            | BenchKind::TextSave
            | BenchKind::Picture(_) => {}
            BenchKind::Completion | BenchKind::Browse | BenchKind::Preview => {}
            BenchKind::SyncView => {
                if self.w9b_scrolling
                    && let Some(st) = folders.get_mut(&view)
                {
                    st.top = (st.top + FOLDER_ROWS_PER_FRAME).min(st.max_top());
                }
            }
            BenchKind::Select if self.scan_done.is_some() => {}
            BenchKind::Folder | BenchKind::FolderColumns | BenchKind::Select => {
                let busy = core
                    .tab(view)
                    .is_some_and(|t| matches!(t.model(), Some(ViewModel::Folder(m)) if m.busy()));
                if matches!(self.script, Script::Waiting(_)) {
                    self.script = Script::Folder;
                }
                if !busy && self.scan_done.is_none() {
                    self.scan_done = Some(Instant::now());
                }
                if let Some(st) = folders.get_mut(&view) {
                    st.top = (st.top + FOLDER_ROWS_PER_FRAME).min(st.max_top());
                }
                if self.scan_done.is_some_and(|t| t.elapsed() >= FOLDER_TAIL) {
                    self.script = Script::Done;
                }
            }
        }
    }

    fn drive_text(&mut self, st: Option<&mut dyn Scrolled>) {
        let Some(st) = st else {
            return;
        };
        match &mut self.script {
            Script::Waiting(n) => {
                if self.ready_screen.is_some() {
                    *n += 1;
                    // `two_windows`: the second window's viewport (its surface and its copy of the font atlas)
                    // is created during the warm-up, which is longer, so the phases measure two open windows.
                    let warm_up = if self.kind == BenchKind::TwoWindows {
                        TWO_WINDOWS_WARM_UP_FRAMES
                    } else {
                        WARM_UP_FRAMES
                    };
                    if *n >= warm_up {
                        self.start_phase(0, st);
                    }
                }
            }
            Script::Phase {
                index, frame, step, ..
            } => {
                let spec = PHASES[*index];
                if *frame >= spec.frames {
                    let next = *index + 1;
                    self.finish_phase(st);
                    if next < PHASES.len() {
                        self.start_phase(next, st);
                    } else {
                        self.script = Script::Tail(0);
                    }
                    return;
                }
                *frame += 1;
                let max = st.max();
                let mut top = *st.top_mut() + *step;
                if top > max + 0.5 {
                    top = if spec.wrap { 0.0 } else { max };
                }
                *st.top_mut() = top.clamp(0.0, max);
            }
            Script::Tail(n) => {
                *n += 1;
                if *n >= SCROLL_TAIL_FRAMES {
                    self.settled_memory = memory();
                    if let Some(obj) = self.settled_memory.as_object_mut() {
                        obj.insert("largest_anon_regions".to_owned(), anon_regions());
                    }
                    self.script = Script::Done;
                }
            }
            Script::Folder | Script::Done => {}
        }
    }

    fn start_phase(&mut self, index: usize, st: &mut dyn Scrolled) {
        let spec = PHASES[index];
        let page = st.page().max(1.0);
        let step = match spec.step {
            Step::Rows(n) => n,
            Step::Page => (page - 1.0).max(1.0),
            Step::Sweep => {
                *st.top_mut() = 0.0;
                (st.max() / f64::from(spec.frames)).max(1.0)
            }
        };
        self.script = Script::Phase {
            index,
            frame: 0,
            started: Instant::now(),
            step,
            samples: Samples::default(),
        };
    }

    fn finish_phase(&mut self, st: &dyn Scrolled) {
        if let Script::Phase {
            index,
            frame,
            started,
            step,
            samples,
        } = &self.script
        {
            let spec = PHASES[*index];
            let seconds = started.elapsed().as_secs_f64();
            let mut phase = json!({
                "name": spec.name,
                "frames": frame,
                "step_rows": round(*step),
                "seconds": round(seconds),
                "fps": (f64::from(*frame) / seconds.max(1e-9) * 10.0).round() / 10.0,
                "visible_rows": st.page(),
            });
            if let (Some(obj), Value::Object(extra)) = (phase.as_object_mut(), samples.json()) {
                obj.extend(extra);
            }
            self.phases.push(phase);
        }
    }

    /// A frame's UI work is done: `work` is the UI thread's time from the start of the frame (messages, jobs,
    /// tree rebuilds, layout, painting commands); the text view tells what it drew.
    pub fn end_frame(
        &mut self,
        texts: &HashMap<ViewId, TextUi>,
        tables: &HashMap<ViewId, TableUi>,
    ) {
        let Some(start) = self.frame_start else {
            return;
        };
        let work = start.elapsed().as_secs_f64() * 1000.0;
        let interval = self
            .last_frame_start
            .map(|last| start.duration_since(last).as_secs_f64() * 1000.0);
        let text = self.view.and_then(|v| texts.get(&v));
        if let Some(st) = text {
            if st.drawn >= 1 && self.first_screen.is_none() {
                self.drawn_pending[0] = Some(());
            }
            if st.drawn >= 2 && self.ready_screen.is_none() {
                self.drawn_pending[1] = Some(());
            }
        }
        // A table view's first drawn rows are both marks: the grid has no unaligned stage.
        if let Some(st) = self.view.and_then(|v| tables.get(&v))
            && st.drawn >= 1
        {
            if self.first_screen.is_none() {
                self.drawn_pending[0] = Some(());
            }
            if self.ready_screen.is_none() {
                self.drawn_pending[1] = Some(());
            }
        }
        let slice = text.and_then(|st| st.slice_ms);
        self.last_work = Some(work);
        self.frames += 1;
        if self.frames == 1 {
            // The window's first frame builds egui's font atlas and pipelines: reported on its own (also the
            // interval that ends with it, in the next frame).
            self.first_frame_work = Some(work);
            return;
        }
        let interval = interval.filter(|_| self.frames > 2);
        push(&mut self.all.work, work);
        if let Some(i) = interval {
            push(&mut self.all.interval, i);
        }
        if let Some(s) = slice {
            push(&mut self.all.slices, s);
        }
        if let Script::Phase { samples, frame, .. } = &mut self.script
            && *frame > 0
        {
            samples.work.push(work);
            if let Some(i) = interval {
                samples.interval.push(i);
            }
            if let Some(s) = slice {
                samples.slices.push(s);
            }
        }
        if let Some(label) = self.w9a_label.take() {
            self.w9a_works.push((label, work));
        }
        if self.kind.scans() && self.scan_done.is_none() {
            push(&mut self.scanning.work, work);
            if let Some(i) = interval {
                push(&mut self.scanning.interval, i);
            }
        }
    }

    /// True once the script finished (the window then reports and quits).
    pub fn finished(&self) -> bool {
        if let Some(h) = &self.hex {
            return h.finished();
        }
        if let Some(p) = &self.picture {
            return p.finished();
        }
        match &self.edit {
            Some(e) => e.finished(),
            None => matches!(self.script, Script::Done),
        }
    }

    /// G16's editing script (M2 W11a): this frame's keys, before the panes draw.
    pub fn drive_edit(
        &mut self,
        ctx: &egui::Context,
        core: &mut AppCore,
        texts: &mut HashMap<ViewId, TextUi>,
    ) {
        let ready = self.ready_screen.is_some();
        if let Some(e) = self.edit.as_mut() {
            let service = core.service_handle();
            e.drive(ctx, core, &service, texts, ready);
        }
    }

    /// G16's editing script: the frame's work goes to its key's row (after [`BenchState::end_frame`]).
    pub fn end_frame_edit(&mut self, core: &AppCore, texts: &HashMap<ViewId, TextUi>) {
        if let (Some(e), Some(work)) = (self.edit.as_mut(), self.last_work.take()) {
            e.end_frame(work, core, texts);
        }
    }

    /// The window's size and scale, for the report.
    pub fn set_window(&mut self, window: Value) {
        if self.window.is_null() {
            self.window = window;
        }
    }

    fn since_start(&self, at: Option<Instant>) -> Value {
        at.map_or(Value::Null, |t| {
            json!(round(
                t.duration_since(self.process_start).as_secs_f64() * 1000.0
            ))
        })
    }

    fn since_view(&self, at: Option<Instant>) -> Value {
        match (self.view_opened, at) {
            (Some(v), Some(t)) => json!(round(t.duration_since(v).as_secs_f64() * 1000.0)),
            _ => Value::Null,
        }
    }

    /// The JSON summary (once; `None` afterwards).
    pub fn final_report(&mut self, core: &AppCore) -> Option<String> {
        if self.reported {
            return None;
        }
        self.reported = true;
        let mut report = json!({
            "bench": match self.kind {
                BenchKind::Scroll => "text_scroll",
                BenchKind::Folder => "folder_scan",
                BenchKind::TwoWindows => "two_windows",
                BenchKind::Workspace => "workspace_restore",
                BenchKind::Options => "options_memory",
                BenchKind::FolderColumns => "folder_columns",
                BenchKind::Select => "select",
                BenchKind::Completion => "completion",
                BenchKind::Browse => "browse",
                BenchKind::Preview => "preview",
                BenchKind::SyncView => "sync_view",
                BenchKind::TableScroll => "table_scroll",
                BenchKind::TextTyping => "text_typing",
                BenchKind::TextUndo => "text_undo",
                BenchKind::TextEdited => "text_edited",
                BenchKind::TextWrap => "text_wrap",
                BenchKind::TextFind => "text_find",
                BenchKind::FindFiles => "find_files",
                BenchKind::TextSave => "text_save",
                BenchKind::Hex(_) => self.hex.as_ref().map_or("hex", |h| h.name()),
                BenchKind::Picture(mode) => mode.name(),
            },
            "windows": core.windows().len(),
            "dialogs": core.windows().iter().map(|w| format!("{:?}", w.dialogs)).collect::<Vec<_>>(),
            "second_window_at": (self.kind == BenchKind::TwoWindows).then_some(
                if self.second_after_coloured { "coloured_screen" } else { "first_screen" }
            ),
            "memory": memory(),
            "memory_at_first_screen": self.first_screen_memory.clone(),
            "memory_settled": self.settled_memory.clone(),
            "tabs": core.windows().iter().map(|w| w.tabs.len()).sum::<usize>(),
            "pending_tabs": core.pending_tabs(),
            "views_built": core.views_built(),
            "renderer": self.renderer.clone(),
            "window": self.window.clone(),
            "first_frame_ms_from_process_start": self.since_start(self.first_frame),
            "first_frame_work_ms": self.first_frame_work.map(round),
            "frames": self.all.json(),
            "view": self.view.and_then(|v| view_summary(core, v)).unwrap_or(Value::Null),
        });
        if let (Some(obj), Some(h)) = (report.as_object_mut(), self.hex.as_ref())
            && let Value::Object(extra) = h.report(self.process_start)
        {
            obj.extend(extra);
        }
        if let Some(obj) = report.as_object_mut() {
            if let Some(p) = &self.picture {
                obj.insert("picture".into(), p.report());
            }
            match self.kind {
                BenchKind::Hex(_) => {}
                BenchKind::Picture(_) => {}
                BenchKind::TextTyping
                | BenchKind::TextUndo
                | BenchKind::TextEdited
                | BenchKind::TextWrap
                | BenchKind::TextFind
                | BenchKind::FindFiles
                | BenchKind::TextSave => {
                    obj.insert(
                        "first_screen_ms".into(),
                        json!({ "from_process_start": self.since_start(self.first_screen) }),
                    );
                    if let Some(e) = &self.edit {
                        obj.insert("edit".into(), e.report());
                    }
                }
                BenchKind::Scroll
                | BenchKind::TwoWindows
                | BenchKind::Workspace
                | BenchKind::TableScroll => {
                    obj.insert(
                        "first_screen_ms".into(),
                        json!({
                            "from_process_start": self.since_start(self.first_screen),
                            "from_view_open": self.since_view(self.first_screen),
                        }),
                    );
                    obj.insert(
                        "coloured_screen_ms".into(),
                        json!({
                            "from_process_start": self.since_start(self.ready_screen),
                            "from_view_open": self.since_view(self.ready_screen),
                        }),
                    );
                    obj.insert("phases".into(), Value::Array(self.phases.clone()));
                }
                BenchKind::Options => {
                    obj.insert("pages".into(), json!(crate::options::pages::PAGES.len()));
                }
                BenchKind::Completion
                | BenchKind::Browse
                | BenchKind::Select
                | BenchKind::Preview
                | BenchKind::SyncView => {
                    obj.insert("plan_ms".into(), json!(self.w9b_plan_ms.map(round)));
                    obj.insert("override_ms".into(), json!(self.w9b_override_ms.map(round)));
                    obj.insert("preview_lines".into(), json!(self.w9b_lines));
                    obj.insert("plan_job_ms".into(), json!(self.w9b_plan_job_ms.map(round)));
                    let mut labels: Vec<&str> = self.w9a_works.iter().map(|(l, _)| *l).collect();
                    labels.dedup();
                    let mut actions = serde_json::Map::new();
                    for label in labels {
                        let works: Vec<f64> = self
                            .w9a_works
                            .iter()
                            .filter(|(l, _)| *l == label)
                            .map(|(_, w)| *w)
                            .collect();
                        actions.insert(label.to_owned(), stats(&works));
                    }
                    obj.insert("actions".into(), Value::Object(actions));
                    obj.insert("done_ms".into(), json!(self.w9a_done_ms.map(round)));
                    obj.insert(
                        "scan_ms_from_view_open".into(),
                        self.since_view(self.scan_done),
                    );
                }
                BenchKind::Folder | BenchKind::FolderColumns => {
                    obj.insert(
                        "scan_ms_from_view_open".into(),
                        self.since_view(self.scan_done),
                    );
                    let mut scanning = self.scanning.json();
                    if let Some(s) = scanning.as_object_mut() {
                        s.insert("frames".into(), json!(self.scanning.work.len()));
                    }
                    obj.insert("while_scanning".into(), scanning);
                }
            }
        }
        serde_json::to_string(&report).ok()
    }
}

/// The folder view `view` of `core`.
fn folder_of(core: &mut AppCore, view: ViewId) -> Option<&mut crate::folder_view::FolderViewModel> {
    match core.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Folder(m)) => Some(m),
        _ => None,
    }
}

fn push(v: &mut Vec<f64>, x: f64) {
    if v.len() < 1_000_000 {
        v.push(x);
    }
}

pub(crate) fn round(ms: f64) -> f64 {
    (ms * 100.0).round() / 100.0
}

/// Count, mean, p50, p95, p99, max and the share above one 60 Hz frame (16.7 ms) of `samples` (ms).
pub fn stats(samples: &[f64]) -> Value {
    if samples.is_empty() {
        return json!({ "count": 0 });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let pick = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let over = sorted.iter().filter(|&&x| x > 1000.0 / 60.0).count();
    json!({
        "count": sorted.len(),
        "mean": round(sorted.iter().sum::<f64>() / sorted.len() as f64),
        "p50": round(pick(0.5)),
        "p95": round(pick(0.95)),
        "p99": round(pick(0.99)),
        "max": round(pick(1.0)),
        "over_16_7ms": over,
    })
}

/// Facts about the view under test (rows, timings, rebuilds).
fn view_summary(core: &AppCore, view: ViewId) -> Option<Value> {
    let tab = core.tab(view)?;
    Some(match tab.model()? {
        ViewModel::Hex(m) => json!({
            "kind": "hex",
            "rows": m.row_count(),
            "columns": m.columns(),
            "aligned": m.alignment().is_some(),
        }),
        ViewModel::Picture(m) => json!({
            "kind": "picture",
            "area": m.comparison().map(|c| [c.area().width, c.area().height]),
            "texture_bytes": m.tiles.bytes(),
        }),
        ViewModel::Table(m) => json!({
            "kind": "table",
            "rows": m.response().map_or(0, |r| r.comparison.sheets().iter().map(|s| s.rows().len()).sum::<usize>()),
        }),
        ViewModel::Text(m) => {
            let (loaded, ready) = m.timings();
            let rows = m.response().map_or(0, |r| r.comparison.row_count());
            let lines = m.response().map_or((0, 0), |r| {
                (
                    r.comparison.document(hikasa_core::Side::Left).line_count(),
                    r.comparison.document(hikasa_core::Side::Right).line_count(),
                )
            });
            json!({
                "rows": rows,
                "lines": [lines.0, lines.1],
                "format": m.response().map(|r| r.format.name.clone()),
                "algorithm": m.response().map(|r| format!("{:?}", r.comparison.algorithm())),
                "documents_loaded_ms": loaded.map(|d| d.as_millis() as u64),
                "comparison_done_ms": ready.map(|d| d.as_millis() as u64),
            })
        }
        ViewModel::Folder(m) => {
            let rebuild = m.rebuild_stats();
            let files = m.prepared().map(|p| {
                let read = p.model().read();
                let s = read.stats(false);
                (s.left.files, s.right.files)
            });
            json!({
                "visible_rows": rebuild.rows,
                "rebuilds_while_scanning": rebuild.during_scan,
                "rebuild_ms": stats(&rebuild.samples),
                "scan_events": m.events(),
                "files": files.map(|f| [f.0, f.1]),
                "scan_ms": m.finished_after().map(|d| d.as_millis() as u64),
            })
        }
        ViewModel::Home(_) => Value::Null,
    })
}

/// The ARCH-GUI-003 memory limit: 400 MB (decimal) in MiB.
pub const LIMIT_MIB: f64 = 400.0 * 1_000_000.0 / 1_048_576.0;

/// Memory of this process and of any descendants, in MiB (1 MiB = 1,048,576 bytes). The total is the sum of
/// the *proportional* set sizes (PSS): pages shared with other processes (the graphics driver, system
/// libraries) count with their share; RSS (which counts them in full) is kept alongside, and the GUI
/// process's own RSS is reported separately.
///
/// Sources: PSS and RSS come from one `/proc/<pid>/smaps_rollup` snapshot (exact page walk, so the two are
/// comparable); the peak comes from `VmHWM` in `/proc/<pid>/status`, the only peak the kernel keeps. `VmHWM`
/// (like `VmRSS`) is maintained from per-CPU batched counters, so it is labelled approximate and never
/// compared with the smaps figures.
pub fn memory() -> Value {
    let me = std::process::id();
    let own = status_of(me);
    let mut children = Vec::new();
    let (mut pss, mut rss, mut hwm) = (own.pss_kib, own.rss_kib, own.hwm_kib);
    for pid in descendants(me) {
        let s = status_of(pid);
        pss += s.pss_kib;
        rss += s.rss_kib;
        hwm += s.hwm_kib;
        children.push(json!({
            "pid": pid,
            "name": s.name,
            "pss_mib": mib(s.pss_kib),
            "rss_mib": mib(s.rss_kib),
            "peak_rss_mib": mib(s.hwm_kib),
        }));
    }
    json!({
        "unit": "MiB",
        "limit_mib": round(LIMIT_MIB),
        "gui_rss_mib": mib(own.rss_kib),
        "gui_peak_rss_mib": mib(own.hwm_kib),
        "gui_pss_mib": mib(own.pss_kib),
        "gui_pss_anon_mib": mib(own.pss_anon_kib),
        "own_mib": own_memory().map(mib),
        "total_pss_mib": mib(pss),
        "total_rss_mib": mib(rss),
        "total_peak_rss_mib": mib(hwm),
        "processes": children,
    })
}

/// Anonymous memory plus the pages of this binary, counted whole, from `/proc/self/smaps`. PSS shares a library's pages
/// out among the processes that map it, so with a software renderer the same memory counts differently by who else runs
/// (docs/02, 0.1.91); this leaves libraries out and depends on nobody else, which makes it the figure to compare between
/// two runs. A measure beside the gates, not one of them.
fn own_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/smaps").ok()?;
    let exe = std::env::current_exe().ok()?;
    Some(own_of_smaps(&text, &exe.to_string_lossy()))
}

/// See [`own_memory`]: `exe` is the path of the binary in `smaps`.
fn own_of_smaps(smaps: &str, exe: &str) -> u64 {
    let mut total = 0;
    let mut in_exe = false;
    for line in smaps.lines() {
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or_default();
        if first.contains('-') && !first.ends_with(':') {
            in_exe = words.nth(4) == Some(exe);
        } else if first == "Anonymous:" {
            total += kib_field(line, "Anonymous:").unwrap_or(0);
        } else if first == "Rss:" && in_exe {
            total += kib_field(line, "Rss:").unwrap_or(0);
        }
    }
    total
}

/// The largest anonymous regions of this process (`/proc/self/smaps`), to tell the heap from buffers: what a
/// difference between two runs is made of.
fn anon_regions() -> Value {
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps") else {
        return Value::Null;
    };
    let mut regions: Vec<(u64, String)> = Vec::new();
    let mut name = String::new();
    for line in text.lines() {
        if line
            .split_whitespace()
            .next()
            .is_some_and(|w| w.contains('-') && !w.ends_with(':'))
        {
            name = line
                .split_whitespace()
                .nth(5)
                .unwrap_or("[anon]")
                .to_owned();
        } else if let Some(rest) = line.strip_prefix("Anonymous:") {
            let kib: u64 = rest
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            regions.push((kib, name.clone()));
        }
    }
    regions.sort_by_key(|r| std::cmp::Reverse(r.0));
    regions.truncate(8);
    regions
        .into_iter()
        .map(|(kib, name)| json!({ "mib": mib(kib), "name": name }))
        .collect()
}

fn mib(kib: u64) -> f64 {
    round(kib as f64 / 1024.0)
}

#[derive(Default)]
struct ProcStatus {
    name: String,
    rss_kib: u64,
    hwm_kib: u64,
    pss_kib: u64,
    pss_anon_kib: u64,
}

/// `Name` and `VmHWM` (approximate peak) from `/proc/<pid>/status`; `Rss`, `Pss` and `Pss_Anon` from one
/// `/proc/<pid>/smaps_rollup` snapshot.
fn status_of(pid: u32) -> ProcStatus {
    let mut out = ProcStatus::default();
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        parse_status(&text, &mut out);
    }
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        parse_rollup(&text, &mut out);
    }
    out
}

fn parse_status(text: &str, out: &mut ProcStatus) {
    if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name:")) {
        out.name = name.trim().to_owned();
    }
    out.hwm_kib = kib_field(text, "VmHWM:").unwrap_or(0);
}

fn parse_rollup(text: &str, out: &mut ProcStatus) {
    out.rss_kib = kib_field(text, "Rss:").unwrap_or(0);
    out.pss_kib = kib_field(text, "Pss:").unwrap_or(0);
    out.pss_anon_kib = kib_field(text, "Pss_Anon:").unwrap_or(0);
}

/// The kB value of the line starting with `key` (`Pss:   1234 kB`).
fn kib_field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

/// Every descendant process of `root` (Linux `/proc`; empty elsewhere).
fn descendants(root: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut parent_of = Vec::new();
    for entry in entries.map_while(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`: comm may contain spaces, so read after the last ')'.
        let ppid = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u32>().ok());
        if let Some(ppid) = ppid {
            parent_of.push((pid, ppid));
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for &(pid, ppid) in &parent_of {
            if ppid == p && pid != root && !out.contains(&pid) {
                out.push(pid);
                frontier.push(pid);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_of_samples() {
        let v = stats(&[1.0, 2.0, 3.0, 4.0, 20.0]);
        assert_eq!(v["count"], 5);
        assert_eq!(v["p50"], 3.0);
        assert_eq!(v["max"], 20.0);
        assert_eq!(v["over_16_7ms"], 1);
        assert_eq!(stats(&[])["count"], 0);
    }

    #[test]
    fn memory_of_this_process() {
        let m = memory();
        if cfg!(target_os = "linux") {
            // Every figure is present; no ordering is asserted between them: PSS and RSS are one smaps
            // snapshot but rounded separately, and the peak comes from a different (batched) kernel counter.
            for key in [
                "gui_rss_mib",
                "gui_pss_mib",
                "gui_peak_rss_mib",
                "total_pss_mib",
            ] {
                assert!(m[key].as_f64().unwrap_or(0.0) > 0.0, "{key}: {m}");
            }
        }
        assert!((LIMIT_MIB - 381.47).abs() < 0.01);
    }

    #[test]
    fn the_own_figure_counts_anonymous_memory_and_the_binary_and_no_library() {
        let smaps = "\
55d0-55e0 r-xp 00000000 00:00 7  /opt/hikasa-gui
Rss:                 100 kB
Pss:                  10 kB
Anonymous:             2 kB
7f00-7f10 r-xp 00000000 00:00 8  /usr/lib/libgallium.so
Rss:               90000 kB
Pss:                 900 kB
Anonymous:             4 kB
7f10-7f20 rw-p 00000000 00:00 0
Rss:                  30 kB
Pss:                  30 kB
Anonymous:            30 kB
";
        assert_eq!(own_of_smaps(smaps, "/opt/hikasa-gui"), 136);
    }

    #[test]
    fn proc_fields_parse() {
        let rollup = "55d0-7ffc ---p 00000000 00:00 0  [rollup]\nRss:              164000 kB\nPss:              117760 kB\nPss_Anon:          90000 kB\n";
        assert_eq!(kib_field(rollup, "Pss:"), Some(117_760));
        assert_eq!(kib_field(rollup, "Pss_Anon:"), Some(90_000));
        assert_eq!(kib_field(rollup, "Pss_File:"), None);
        let mut s = ProcStatus::default();
        parse_status(
            "Name:\thikasa-gui\nVmHWM:\t  2048 kB\nVmRSS:\t  1024 kB\n",
            &mut s,
        );
        // RSS is not taken from the batched `VmRSS` counter.
        assert_eq!(
            (s.name.as_str(), s.rss_kib, s.hwm_kib),
            ("hikasa-gui", 0, 2048)
        );
        parse_rollup(rollup, &mut s);
        assert_eq!(
            (s.rss_kib, s.pss_kib, s.pss_anon_kib),
            (164_000, 117_760, 90_000)
        );
        assert_eq!(mib(117_760), 115.0);
    }
}
