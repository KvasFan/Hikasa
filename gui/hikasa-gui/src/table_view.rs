//! The Table Compare view model (TB-020…TB-024; m2-table-contract §6.6, T-51…): it runs the comparison as a job
//! (`Service::table_compare`, `JobKind::TableCompare`, cancellable), keeps the display filter, the active sheet, the
//! cursor and the scroll position per sheet, the hidden and resized columns, and hands the grid the visible rows,
//! columns and cells (ARCH-GUI-002: the grid lays out only what is on screen; per row the view keeps one `u32`, the
//! index of a row that passes the filter).
//!
//! Every decision about rows, cells, sheets, statistics and navigation is the core's (`TableComparison`,
//! `SheetComparison::cell_state`, `RowFilter`, `nav::*`, `column_differs`): this module maps them to drawing data
//! and never compares a value (architecture check 7).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{ContentKind, JobKind, Side};
use hikasa_session::JobControl;
use hikasa_session::api::{TableCompareRequest, TableCompareResponse, ViewOutcome, ViewRequest};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;
use hikasa_session::{ApiError, Service};
use hikasa_table::{
    CellState, Dir, Importance, RowAlignmentUsed, RowFilter, RowKind, SheetComparison, SheetState,
    TableComparison, next_difference_cell, next_difference_sheet,
};

use crate::commands::{CommandId, Ctx};
use crate::job::Job;
use crate::status::{JobStatus, LogNote, StatusItems};

/// Default font size of the grid (px).
pub const DEFAULT_FONT_PX: u32 = 13;
/// Smallest and largest font sizes.
const FONT_RANGE: (u32, u32) = (8, 32);
/// Default and limits of a column's width, in characters.
pub const COLUMN_CHARS: (f32, f32, f32) = (10.0, 3.0, 80.0);
/// Longest cell text the grid draws (characters); longer texts are cut with a marker.
pub const MAX_CELL_CHARS: usize = 512;
/// Rows *Resize Columns to Fit* measures around the first visible one.
const FIT_ROWS: u32 = 2_000;
/// Most cells a side *Resize Columns to Fit* measures (a wide sheet measures fewer rows).
const FIT_CELLS: usize = 200_000;
/// The scope name of the view type in `state.toml` and in sessions' `[view]`.
const VIEW: &str = "table-compare";

/// Phase of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablePhase {
    /// The comparison runs.
    Loading,
    /// The comparison is ready.
    Ready,
    /// The comparison failed or was cancelled.
    Failed,
}

/// How the two sides are arranged (T-55: both layouts of the file views' *Side-by-Side* / *Over-Under*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridLayout {
    /// The left grid beside the right one.
    #[default]
    SideBySide,
    /// The left grid above the right one.
    OverUnder,
}

impl GridLayout {
    fn word(self) -> &'static str {
        match self {
            GridLayout::SideBySide => "side-by-side",
            GridLayout::OverUnder => "over-under",
        }
    }
}

/// The current cell: a row of the sheet comparison (an index of `SheetComparison::rows`), a comparison column and
/// the side clicked; `whole_row` after a click on a row number (Copy takes the row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Row of the comparison.
    pub row: u32,
    /// Comparison column.
    pub col: u32,
    /// Side.
    pub side: Side,
    /// The whole row is selected.
    pub whole_row: bool,
}

/// What the view keeps per sheet pair.
#[derive(Debug, Clone, Default)]
struct SheetView {
    /// Rows of the comparison that pass the filter, in order (built when the sheet is first shown).
    visible: Option<Vec<u32>>,
    /// The visible rows that differ under the filter's ignore-unimportant (`RowPair::differs`), in order: the
    /// navigation's steps are binary searches of it (review F12), built with `visible`.
    differing: Vec<u32>,
    cursor: Option<Cursor>,
    top: u32,
}

/// One sheet tab (TB-004 spot colours).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetTab {
    /// Caption: the sheet name, `left → right` when the pair was renamed.
    pub caption: String,
    /// State.
    pub state: SheetState,
}

/// One column head of the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnHead {
    /// Comparison column.
    pub col: u32,
    /// Name per side (the header name, else the letters); `None`: the side has no such column.
    pub names: [Option<String>; 2],
    /// Letters of each side's own column (`A`, `B`, …).
    pub letters: [Option<String>; 2],
    /// Key priority (1-based).
    pub key: Option<u8>,
    /// State of the header names.
    pub state: CellState,
    /// Width in characters.
    pub width: f32,
}

/// One row of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRow {
    /// Row of the comparison.
    pub row: u32,
    /// Kind.
    pub kind: RowKind,
    /// Importance (under the view's ignore-unimportant: an unimportant row shows as same when ignored).
    pub importance: Importance,
    /// Row labels (the sheet row or text line) per side.
    pub labels: [Option<u64>; 2],
}

/// One cell of the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridCell {
    /// The display text (`None`: the side has no such row or column).
    pub text: Option<String>,
    /// State (unimportant shows as same while ignored).
    pub state: CellState,
    /// The cell holds a formula.
    pub formula: bool,
}

/// One line of the Text Details panel: a column of the current row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailLine {
    /// Column name.
    pub name: String,
    /// Left and right display texts (with the formula after `=` when there is one).
    pub texts: [Option<String>; 2],
    /// State.
    pub state: CellState,
}

/// What runs a view's comparison: `Service::table_compare` (tests put a job of their own in its place).
pub(crate) type Runner = Arc<
    dyn Fn(&Service, &JobControl, &TableCompareRequest) -> Result<TableCompareResponse, ApiError>
        + Send
        + Sync,
>;

/// The Table Compare view model.
pub struct TableViewModel {
    request: TableCompareRequest,
    run: Runner,
    titles: [Option<String>; 2],
    names: [String; 2],
    wake: Arc<dyn Fn() + Send + Sync>,
    job: Option<Job<TableCompareResponse>>,
    response: Option<TableCompareResponse>,
    error: Option<String>,
    cancelled: bool,
    filter: RowFilter,
    sheet: usize,
    sheets: Vec<SheetView>,
    /// Hidden columns by sheet and column key (`[view] hidden_columns`; the key is the column's name, `name#2` for
    /// its second occurrence, review F10).
    hidden: BTreeSet<(String, String)>,
    /// Widths in characters by sheet and column key (`[view] widths`).
    widths: BTreeMap<(String, String), f32>,
    /// The column keys of the active sheet, per comparison column (built with the sheet, review F10).
    keys: Vec<String>,
    /// The column heads of the active sheet, per comparison column: built when the sheet, the filter or a width
    /// changes, never per frame (review F12).
    heads: Vec<ColumnHead>,
    /// `shown_mask[col]`: comparison column `col` is shown (with `shown`; review F11, F12).
    shown_mask: Vec<bool>,
    /// Bumped when the shown columns or a width change: what the grid keeps of the columns' positions is stale.
    columns_generation: u64,
    /// The sheet the session showed (`[view] sheet`), until the comparison is ready.
    wanted_sheet: Option<String>,
    hide_same: bool,
    show_thumbnail: bool,
    show_details: bool,
    row_numbers: bool,
    layout: GridLayout,
    font_px: u32,
    default_font_px: u32,
    /// The comparison columns shown, in order (the active sheet; rebuilt on every change).
    shown: Vec<u32>,
    /// The page the grid shows (rows).
    page: u32,
    /// One-shot scroll request: a position of the visible rows to bring into view.
    scroll: Option<u32>,
    /// Next / previous difference exist from the cursor (computed on change, not per frame).
    nav: (bool, bool),
    generation: u64,
    notes: Vec<LogNote>,
    started: Instant,
    ready_after: Option<Duration>,
    write: crate::folder_view::WritePolicy,
}

impl TableViewModel {
    /// Creates the view and starts its comparison job. `wake` posts a wake-up to the UI thread.
    pub fn new(
        service: &Service,
        request: TableCompareRequest,
        filter: RowFilter,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> TableViewModel {
        let run: Runner = Arc::new(
            |svc: &Service, job: &JobControl, request: &TableCompareRequest| {
                svc.table_compare(request, job)
            },
        );
        TableViewModel::with_runner(service, request, filter, titles, wake, run)
    }

    /// [`TableViewModel::new`] with the comparison run by `run`.
    fn with_runner(
        service: &Service,
        request: TableCompareRequest,
        filter: RowFilter,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
        run: Runner,
    ) -> TableViewModel {
        let names = [display_name(&request.left), display_name(&request.right)];
        let mut model = TableViewModel {
            request,
            run,
            titles,
            names,
            wake,
            job: None,
            response: None,
            error: None,
            cancelled: false,
            filter,
            sheet: 0,
            sheets: Vec::new(),
            hidden: BTreeSet::new(),
            widths: BTreeMap::new(),
            keys: Vec::new(),
            heads: Vec::new(),
            shown_mask: Vec::new(),
            columns_generation: 0,
            wanted_sheet: None,
            hide_same: false,
            show_thumbnail: true,
            show_details: false,
            row_numbers: true,
            layout: GridLayout::SideBySide,
            font_px: DEFAULT_FONT_PX,
            default_font_px: DEFAULT_FONT_PX,
            shown: Vec::new(),
            page: 40,
            scroll: None,
            nav: (false, false),
            generation: 1,
            notes: Vec::new(),
            started: Instant::now(),
            ready_after: None,
            write: crate::folder_view::WritePolicy::default(),
        };
        model.start(service);
        model
    }

    /// [`TableViewModel::new`] whose comparison is `run` (tests).
    #[cfg(test)]
    pub(crate) fn with_runner_for_test(
        service: &Service,
        request: TableCompareRequest,
        filter: RowFilter,
        titles: [Option<String>; 2],
        wake: Arc<dyn Fn() + Send + Sync>,
        run: Runner,
    ) -> TableViewModel {
        TableViewModel::with_runner(service, request, filter, titles, wake, run)
    }

    /// Sets the cursor (tests).
    #[cfg(test)]
    pub(crate) fn set_cursor_for_test(&mut self, cursor: Option<Cursor>) {
        self.set_cursor(cursor);
    }

    /// Sets what the view may write (FC-003; a table view writes nothing, its session keeps the sides).
    pub fn set_write_policy(&mut self, write: crate::folder_view::WritePolicy) {
        self.write = write;
    }

    /// What the view may write.
    pub fn write_policy(&self) -> crate::folder_view::WritePolicy {
        self.write
    }

    /// (Re)starts the comparison (Reload): a job of `JobKind::TableCompare`.
    fn start(&mut self, service: &Service) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
        if let Some(name) = self.sheet_name(self.sheet) {
            self.wanted_sheet.get_or_insert(name);
        }
        self.response = None;
        self.error = None;
        self.cancelled = false;
        self.sheets.clear();
        self.shown.clear();
        self.shown_mask.clear();
        self.heads.clear();
        self.keys.clear();
        self.started = Instant::now();
        self.ready_after = None;
        self.bump();
        let request = self.request.clone();
        let run = Arc::clone(&self.run);
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        self.job = Some(Job::spawn(
            service,
            JobKind::TableCompare,
            title,
            &self.wake,
            move |svc, job| run(svc, job, &request),
        ));
    }

    /// The comparison request.
    pub fn request(&self) -> &TableCompareRequest {
        &self.request
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&self.names[0], &self.names[1]],
        )
    }

    /// The finished comparison, if any.
    pub fn response(&self) -> Option<&TableCompareResponse> {
        self.response.as_ref()
    }

    /// The finished comparison under the view's ignore-unimportant (what its report shows; review F6).
    pub fn report_response(&self) -> Option<TableCompareResponse> {
        let mut r = self.response.clone()?;
        let iu = self.iu();
        r.ignore_unimportant = iu;
        r.kind = r.comparison.kind(r.bytes_identical, iu);
        r.exit_code = hikasa_core::ExitCode::for_kind(r.kind, iu);
        Some(r)
    }

    fn comparison(&self) -> Option<&TableComparison> {
        self.response.as_ref().map(|r| &*r.comparison)
    }

    fn sheet_cmp(&self) -> Option<&SheetComparison> {
        self.comparison()?.sheets().get(self.sheet)
    }

    /// The display filter.
    pub fn filter(&self) -> RowFilter {
        self.filter
    }

    /// True while the comparison runs.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// True while the job has ended but its result is not published yet.
    pub fn settling(&self) -> bool {
        self.job.as_ref().is_some_and(Job::settling)
    }

    /// Stops the comparison (the view is closing).
    pub fn cancel(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }

    /// The data generation: what the grid caches of another generation is stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn bump(&mut self) {
        self.generation += 1;
    }

    /// Polls the job (once per frame); true when something changed.
    pub fn poll(&mut self, _service: &Service) -> bool {
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        match result {
            Ok(response) => {
                self.finish(response);
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
        for w in self.response.iter().flat_map(|r| &r.warnings) {
            self.notes.push(LogNote {
                kind: LogKind::Warning,
                text: format!("[{}] {}", w.code.as_str(), w.message),
            });
        }
        true
    }

    fn finish(&mut self, response: TableCompareResponse) {
        self.ready_after = Some(self.started.elapsed());
        let count = response.comparison.sheets().len();
        self.sheets = vec![SheetView::default(); count];
        let wanted = self.wanted_sheet.take();
        self.response = Some(response);
        self.sheet = wanted.and_then(|name| self.sheet_index(&name)).unwrap_or(0);
        self.activate_sheet();
    }

    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.comparison()?
            .sheets()
            .iter()
            .position(|s| s.name() == name)
    }

    fn sheet_name(&self, k: usize) -> Option<String> {
        Some(self.comparison()?.sheets().get(k)?.name().to_owned())
    }

    /// Builds the active sheet's rows and columns (first show, a filter change) and its navigation.
    fn activate_sheet(&mut self) {
        let Some(cmp) = self.response.as_ref().map(|r| Arc::clone(&r.comparison)) else {
            return;
        };
        let Some(sheet) = cmp.sheets().get(self.sheet) else {
            self.bump();
            return;
        };
        let filter = self.filter;
        if let Some(view) = self.sheets.get_mut(self.sheet)
            && view.visible.is_none()
        {
            let visible: Vec<u32> = sheet.visible(&filter).collect();
            let rows = sheet.rows();
            view.differing = visible
                .iter()
                .copied()
                .filter(|&r| {
                    rows.get(r as usize)
                        .is_some_and(|p| p.differs(filter.ignore_unimportant))
                })
                .collect();
            view.visible = Some(visible);
        }
        self.rebuild_heads();
        self.rebuild_columns();
        let top = self.sheets.get(self.sheet).map_or(0, |v| v.top);
        self.scroll = Some(top);
        self.refresh_nav();
        self.bump();
    }

    /// The column keys and heads of the active sheet (a sheet change, a filter change, new widths).
    fn rebuild_heads(&mut self) {
        self.columns_generation += 1;
        self.keys = self
            .sheet_cmp()
            .map(|sheet| column_keys(sheet.columns().iter().map(|c| c.name.as_str())))
            .unwrap_or_default();
        self.heads = match self.sheet_cmp() {
            Some(sheet) => (0..sheet.columns().len() as u32)
                .filter_map(|col| self.make_head(col))
                .collect(),
            None => Vec::new(),
        };
    }

    /// Recomputes the shown columns of the active sheet (hidden by the session, by `--hide`, by Hide Same Columns).
    fn rebuild_columns(&mut self) {
        self.columns_generation += 1;
        let Some(sheet) = self.sheet_cmp() else {
            self.shown.clear();
            self.shown_mask.clear();
            return;
        };
        let differing = self
            .hide_same
            .then(|| sheet.differing_columns(self.filter.ignore_unimportant));
        let name = sheet.name();
        let hidden: BTreeSet<&str> = self
            .hidden
            .iter()
            .filter(|(s, _)| s == name)
            .map(|(_, c)| c.as_str())
            .collect();
        let shown: Vec<u32> = sheet
            .columns()
            .iter()
            .enumerate()
            .filter(|(k, c)| {
                !c.hidden
                    && !self
                        .keys
                        .get(*k)
                        .is_some_and(|key| hidden.contains(key.as_str()))
                    && differing
                        .as_ref()
                        .is_none_or(|d| d.get(*k).copied().unwrap_or(true) || c.key.is_some())
            })
            .map(|(k, _)| k as u32)
            .collect();
        let mut mask = vec![false; sheet.columns().len()];
        for &c in &shown {
            mask[c as usize] = true;
        }
        self.shown = shown;
        self.shown_mask = mask;
    }

    /// True when comparison column `col` is shown.
    fn is_shown(&self, col: u32) -> bool {
        self.shown_mask.get(col as usize).copied().unwrap_or(false)
    }

    /// Rebuilds the visible rows of every sheet after a filter change (the active one now, the others when shown).
    fn refilter(&mut self) {
        let cursor_row = self.cursor().map(|c| c.row);
        for view in &mut self.sheets {
            view.visible = None;
            view.differing = Vec::new();
        }
        self.activate_sheet();
        // The cursor's row, or the nearest shown after it, stays in view.
        if let Some(row) = cursor_row {
            let pos = self.position_at_or_after(row);
            self.scroll = Some(pos);
        }
    }

    fn visible(&self) -> &[u32] {
        self.sheets
            .get(self.sheet)
            .and_then(|v| v.visible.as_deref())
            .unwrap_or(&[])
    }

    /// The position of `row` among the visible rows, or of the first shown after it.
    fn position_at_or_after(&self, row: u32) -> u32 {
        self.visible().partition_point(|&r| r < row) as u32
    }

    /// The position of `row` when it is shown.
    pub fn position_of(&self, row: u32) -> Option<u32> {
        self.visible().binary_search(&row).ok().map(|p| p as u32)
    }

    /// The phase.
    pub fn phase(&self) -> TablePhase {
        if self.response.is_some() {
            TablePhase::Ready
        } else if self.error.is_some() || self.cancelled {
            TablePhase::Failed
        } else {
            TablePhase::Loading
        }
    }

    /// Text for an empty grid (loading, errors, "no rows shown").
    pub fn message(&self) -> Option<String> {
        match self.phase() {
            TablePhase::Loading => Some("Comparing…".to_owned()),
            TablePhase::Failed => Some(
                self.error
                    .clone()
                    .unwrap_or_else(|| "The comparison was cancelled.".to_owned()),
            ),
            TablePhase::Ready => match self.sheet_cmp() {
                None => Some("Neither side has a sheet.".to_owned()),
                Some(s) if s.rows().is_empty() => Some("The sheets have no rows.".to_owned()),
                Some(_) if self.len() == 0 => {
                    Some("No rows are shown by the display filter.".to_owned())
                }
                Some(_) => None,
            },
        }
    }

    /// Rows shown (positions).
    pub fn len(&self) -> u32 {
        self.visible().len() as u32
    }

    /// The sheet tabs.
    pub fn sheet_tabs(&self) -> Vec<SheetTab> {
        let Some(cmp) = self.comparison() else {
            return Vec::new();
        };
        cmp.sheets()
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let other = if s.renamed() {
                    cmp.sheet(k, Side::Right).map(|r| r.name().to_owned())
                } else {
                    None
                };
                SheetTab {
                    caption: match other {
                        Some(right) => format!("{} → {right}", s.name()),
                        None => s.name().to_owned(),
                    },
                    // Under ignore-unimportant a sheet of unimportant differences is a same sheet (review F13).
                    state: match s.state() {
                        SheetState::Unimportant if self.iu() => SheetState::Same,
                        state => state,
                    },
                }
            })
            .collect()
    }

    /// The active sheet pair.
    pub fn active_sheet(&self) -> usize {
        self.sheet
    }

    /// Shows sheet pair `k` (a click on its tab).
    pub fn set_sheet(&mut self, k: usize) {
        if k == self.sheet || k >= self.sheets.len() {
            return;
        }
        self.sheet = k;
        self.activate_sheet();
    }

    /// The comparison columns shown, in order.
    pub fn shown_columns(&self) -> &[u32] {
        &self.shown
    }

    /// Changes when the shown columns or a column's width change (the grid keeps their positions until then).
    pub fn columns_generation(&self) -> u64 {
        self.columns_generation
    }

    /// The head of comparison column `col` (kept per sheet, filter and widths; review F12).
    pub fn column_head(&self, col: u32) -> Option<&ColumnHead> {
        self.heads.get(col as usize)
    }

    fn make_head(&self, col: u32) -> Option<ColumnHead> {
        let cmp = self.comparison()?;
        let sheet = cmp.sheets().get(self.sheet)?;
        let pair = sheet.columns().get(col as usize)?;
        let side = |side: Side, own: Option<u32>| -> (Option<String>, Option<String>) {
            let (Some(c), Some(s)) = (own, cmp.sheet(self.sheet, side)) else {
                return (None, None);
            };
            let name = s
                .header()
                .and_then(|h| h.get(c as usize))
                .filter(|_| sheet.has_header(side))
                .cloned()
                .unwrap_or_else(|| s.column_label(c));
            (Some(name), Some(s.column_label(c)))
        };
        let (ln, ll) = side(Side::Left, pair.left);
        let (rn, rl) = side(Side::Right, pair.right);
        let state = sheet
            .header_states()
            .get(col as usize)
            .copied()
            .unwrap_or(CellState::Same);
        Some(ColumnHead {
            col,
            names: [ln, rn],
            letters: [ll, rl],
            // The engine counts key priorities from 0.
            key: pair.key.map(|k| k.saturating_add(1)),
            state: self.shown_state(state),
            width: self.width_of(sheet.name(), col, &pair.name),
        })
    }

    /// The width of comparison column `col` named `column` (the session's, else from the name).
    fn width_of(&self, sheet: &str, col: u32, column: &str) -> f32 {
        self.keys
            .get(col as usize)
            .and_then(|key| self.widths.get(&(sheet.to_owned(), key.clone())))
            .copied()
            .unwrap_or_else(|| (column.chars().count() as f32 + 2.0).clamp(COLUMN_CHARS.0, 24.0))
    }

    /// The `(sheet, column key)` of comparison column `col` of the active sheet.
    fn column_key(&self, col: u32) -> Option<(String, String)> {
        let sheet = self.sheet_cmp()?;
        Some((
            sheet.name().to_owned(),
            self.keys.get(col as usize)?.clone(),
        ))
    }

    /// Unimportant shows as same while ignored.
    fn shown_state(&self, state: CellState) -> CellState {
        if self.filter.ignore_unimportant && state == CellState::Unimportant {
            CellState::Same
        } else {
            state
        }
    }

    /// The row shown at position `pos`.
    pub fn row_at(&self, pos: u32) -> Option<GridRow> {
        let row = *self.visible().get(pos as usize)?;
        let cmp = self.comparison()?;
        let pair = self.sheet_cmp()?.rows().get(row as usize)?;
        let importance = match pair.importance() {
            Importance::Unimportant if self.filter.ignore_unimportant => Importance::None,
            other => other,
        };
        Some(GridRow {
            row,
            kind: pair.kind(),
            importance,
            labels: [
                cmp.row_label(self.sheet, row, Side::Left),
                cmp.row_label(self.sheet, row, Side::Right),
            ],
        })
    }

    /// The cell of comparison row `row`, column `col`, on `side`.
    pub fn cell(&self, row: u32, col: u32, side: Side) -> GridCell {
        let absent = GridCell {
            text: None,
            state: CellState::Same,
            formula: false,
        };
        let (Some(cmp), Some(sheet)) = (self.comparison(), self.sheet_cmp()) else {
            return absent;
        };
        let (Some(pair), Some(column)) = (
            sheet.rows().get(row as usize),
            sheet.columns().get(col as usize),
        ) else {
            return absent;
        };
        let own = match side {
            Side::Left => column.left,
            _ => column.right,
        };
        if pair.side(side).is_none() || own.is_none() {
            return GridCell {
                state: self.shown_state(sheet.cell_state(row, col)),
                ..absent
            };
        }
        let value = cmp.cell(self.sheet, row, col, side);
        GridCell {
            text: Some(cut(&value.display())),
            state: self.shown_state(sheet.cell_state(row, col)),
            formula: cmp.formula(self.sheet, row, col, side).is_some(),
        }
    }

    /// The thumbnail (TB-020): one byte per bucket of the shown rows, 0 same, 1 unimportant, 2 important, 3 a row
    /// of one side; a bucket takes its most important row.
    pub fn thumbnail(&self, height: u32) -> Vec<u8> {
        let rows = self.visible();
        let Some(sheet) = self.sheet_cmp() else {
            return Vec::new();
        };
        let height = height.max(1) as usize;
        let mut cells = vec![0u8; height.min(rows.len().max(1))];
        let n = cells.len();
        let pairs = sheet.rows();
        for (pos, &row) in rows.iter().enumerate() {
            let Some(pair) = pairs.get(row as usize) else {
                continue;
            };
            let code = match (pair.kind(), pair.importance()) {
                (_, Importance::None) => 0,
                (_, Importance::Unimportant) if self.filter.ignore_unimportant => 0,
                (RowKind::LeftOnly | RowKind::RightOnly, _) => 3,
                (_, Importance::Important) => 2,
                (_, Importance::Unimportant) => 1,
            };
            let bucket = pos * n / rows.len().max(1);
            if let Some(cell) = cells.get_mut(bucket) {
                *cell = (*cell).max(code);
            }
        }
        cells
    }

    /// The grid reports its viewport (first visible position, rows on screen).
    pub fn set_viewport(&mut self, top: u32, page: u32) {
        let len = self.len();
        if let Some(view) = self.sheets.get_mut(self.sheet) {
            view.top = top.min(len);
        }
        self.page = page.max(1);
    }

    /// The first visible position the view keeps.
    pub fn top(&self) -> u32 {
        self.sheets.get(self.sheet).map_or(0, |v| v.top)
    }

    /// The one-shot scroll request (a position to bring into view).
    pub fn take_scroll(&mut self) -> Option<u32> {
        self.scroll.take()
    }

    /// The cursor.
    pub fn cursor(&self) -> Option<Cursor> {
        self.sheets.get(self.sheet).and_then(|v| v.cursor)
    }

    fn set_cursor(&mut self, cursor: Option<Cursor>) {
        if let Some(view) = self.sheets.get_mut(self.sheet) {
            view.cursor = cursor;
        }
        self.refresh_nav();
    }

    /// A click on a cell (`col` a comparison column) on `side`.
    pub fn click(&mut self, pos: u32, col: u32, side: Side) {
        let Some(&row) = self.visible().get(pos as usize) else {
            return;
        };
        self.set_cursor(Some(Cursor {
            row,
            col,
            side,
            whole_row: false,
        }));
    }

    /// Selects the whole row at `pos` on `side` (a click on its row number).
    pub fn click_row(&mut self, pos: u32, side: Side) {
        let Some(&row) = self.visible().get(pos as usize) else {
            return;
        };
        let col = self
            .cursor()
            .map(|c| c.col)
            .or_else(|| self.shown.first().copied())
            .unwrap_or(0);
        self.set_cursor(Some(Cursor {
            row,
            col,
            side,
            whole_row: true,
        }));
    }

    /// Arrow keys: moves the cursor by shown rows and shown columns; Tab-less, no wrap.
    pub fn move_cursor(&mut self, rows: i64, cols: i64) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let cursor = self.cursor();
        let pos = cursor
            .and_then(|c| self.position_of(c.row))
            .map_or(i64::from(self.top()), i64::from);
        let pos = (pos + rows).clamp(0, i64::from(len) - 1) as u32;
        let col_at = cursor
            .and_then(|c| self.shown.iter().position(|&s| s == c.col))
            .unwrap_or(0) as i64;
        let col_at = (col_at + cols).clamp(0, (self.shown.len() as i64 - 1).max(0)) as usize;
        let Some(&row) = self.visible().get(pos as usize) else {
            return;
        };
        let col = self.shown.get(col_at).copied().unwrap_or(0);
        let side = cursor.map_or(Side::Left, |c| c.side);
        self.set_cursor(Some(Cursor {
            row,
            col,
            side,
            whole_row: false,
        }));
        self.scroll = Some(pos);
    }

    /// Changes a column's width (characters; the grid's drag on a head's edge).
    pub fn set_width(&mut self, col: u32, chars: f32) {
        let Some(key) = self.column_key(col) else {
            return;
        };
        let chars = chars.clamp(COLUMN_CHARS.1, COLUMN_CHARS.2);
        self.columns_generation += 1;
        self.widths.insert(key, chars);
        if let Some(head) = self.heads.get_mut(col as usize) {
            head.width = chars;
        }
    }

    /// Hides a column of the active sheet (the head's context menu); the session keeps it.
    pub fn hide_column(&mut self, col: u32) {
        let Some(key) = self.column_key(col) else {
            return;
        };
        self.hidden.insert(key);
        self.rebuild_columns();
        self.bump();
    }

    /// Shows every hidden column of the active sheet again.
    pub fn show_all_columns(&mut self) {
        let Some(name) = self.sheet_name(self.sheet) else {
            return;
        };
        self.hidden.retain(|(sheet, _)| *sheet != name);
        self.rebuild_columns();
        self.bump();
    }

    /// The columns the session hid in the active sheet (head menu: *Show Hidden Columns*).
    pub fn hidden_count(&self) -> usize {
        let Some(name) = self.sheet_name(self.sheet) else {
            return 0;
        };
        self.hidden
            .iter()
            .filter(|(sheet, _)| *sheet == name)
            .count()
    }

    /// Resize Columns to Fit (TB-020): every shown column as wide as its widest text among the head and the rows
    /// around the first visible one ([`FIT_ROWS`]; a million rows are not measured).
    pub fn fit_columns(&mut self) {
        let Some(cmp) = self.response.as_ref().map(|r| Arc::clone(&r.comparison)) else {
            return;
        };
        if cmp.sheets().get(self.sheet).is_none() {
            return;
        }
        // Rows around the first visible one, fewer on a wide sheet: at most `FIT_CELLS` cells a side are measured.
        let take = (FIT_CELLS / self.shown.len().max(1)).clamp(1, FIT_ROWS as usize);
        let top = self.top() as usize;
        let rows: Vec<u32> = self
            .visible()
            .iter()
            .skip(top.saturating_sub(take / 2))
            .take(take)
            .copied()
            .collect();
        let shown = self.shown.clone();
        let mut widest: Vec<usize> = shown
            .iter()
            .map(|&col| {
                self.column_head(col).map_or(0, |h| {
                    h.names
                        .iter()
                        .flatten()
                        .map(|n| n.chars().count())
                        .max()
                        .unwrap_or(0)
                })
            })
            .collect();
        // Row by row: a text sheet re-reads a record once for its cells (T-65).
        for &row in &rows {
            for side in [Side::Left, Side::Right] {
                for (k, &col) in shown.iter().enumerate() {
                    let n = cmp
                        .cell(self.sheet, row, col, side)
                        .display()
                        .chars()
                        .count();
                    widest[k] = widest[k].max(n.min(MAX_CELL_CHARS));
                }
            }
        }
        for (k, &col) in shown.iter().enumerate() {
            self.set_width(col, widest[k] as f32 + 2.0);
        }
        self.bump();
    }

    /// The current row's cells (the Text Details panel, TB-020): the first `limit` lines (the panel's; review F12)
    /// and how many columns there are.
    pub fn details(&self, limit: usize) -> (Vec<DetailLine>, usize) {
        let (Some(cmp), Some(sheet), Some(cursor)) =
            (self.comparison(), self.sheet_cmp(), self.cursor())
        else {
            return (Vec::new(), 0);
        };
        let lines = sheet
            .columns()
            .iter()
            .take(limit)
            .enumerate()
            .map(|(k, c)| {
                let col = k as u32;
                let text = |side: Side| {
                    let cell = self.cell(cursor.row, col, side);
                    cell.text
                        .map(|t| match cmp.formula(self.sheet, cursor.row, col, side) {
                            Some(f) => format!("{t}    ={f}"),
                            None => t,
                        })
                };
                DetailLine {
                    name: c.name.clone(),
                    texts: [text(Side::Left), text(Side::Right)],
                    state: self.shown_state(sheet.cell_state(cursor.row, col)),
                }
            })
            .collect();
        (lines, sheet.columns().len())
    }

    /// The full display text of a cell (`None`: the side has no such row or column): what Copy places (review F5).
    fn full_text(&self, row: u32, col: u32, side: Side) -> Option<String> {
        let (cmp, sheet) = (self.comparison()?, self.sheet_cmp()?);
        let (pair, column) = (
            sheet.rows().get(row as usize)?,
            sheet.columns().get(col as usize)?,
        );
        let own = match side {
            Side::Left => column.left,
            _ => column.right,
        };
        pair.side(side)?;
        own?;
        Some(cmp.cell(self.sheet, row, col, side).display().into_owned())
    }

    /// What Copy places (TB-023's copy of a cell or a row): the current cell's full display text (not the grid's
    /// one-line, cut drawing of it; review F5), or after a click on its row number the cursor side's row: the shown
    /// columns' texts (hidden columns left out, R12), tab-separated, a text holding a tab, a line break or a quote
    /// quoted as CSV/TSV quote it (`"a""b"`).
    pub fn copy_text(&self) -> Option<String> {
        let cursor = self.cursor()?;
        self.sheet_cmp()?;
        if cursor.whole_row {
            let texts: Vec<String> = self
                .shown
                .iter()
                .map(|&c| {
                    field(
                        &self
                            .full_text(cursor.row, c, cursor.side)
                            .unwrap_or_default(),
                    )
                })
                .collect();
            return Some(texts.join("\t"));
        }
        self.full_text(cursor.row, cursor.col, cursor.side)
    }

    /// The address of the cursor (`B12`: the side's own column letters and row label).
    pub fn address(&self) -> Option<String> {
        let cursor = self.cursor()?;
        let cmp = self.comparison()?;
        let label = cmp.row_label(self.sheet, cursor.row, cursor.side)?;
        let head = self.column_head(cursor.col)?;
        let k = usize::from(cursor.side != Side::Left);
        if cursor.whole_row {
            return Some(format!("row {label}"));
        }
        Some(format!(
            "{}{label}",
            head.letters[k].clone().unwrap_or_default()
        ))
    }

    // ---- navigation ------------------------------------------------------------------------------------------

    fn iu(&self) -> bool {
        self.filter.ignore_unimportant
    }

    fn refresh_nav(&mut self) {
        let from = self.cursor().map(|c| c.row);
        self.nav = (
            self.difference_row(from, Dir::Next).is_some(),
            self.difference_row(from, Dir::Prev).is_some(),
        );
    }

    /// The visible rows that differ (`RowPair::differs` under the filter's ignore-unimportant), in order.
    fn differing(&self) -> &[u32] {
        self.sheets
            .get(self.sheet)
            .map_or(&[][..], |v| v.differing.as_slice())
    }

    /// The next (previous) differing row from `from` that the filter shows: the same rows as
    /// `nav::next_difference_row` skipping the rows the filter hides, found by a binary search of the visible
    /// differing rows (review F12: a step is O(log n) whatever the filter hides).
    fn difference_row(&self, from: Option<u32>, dir: Dir) -> Option<u32> {
        let rows = self.differing();
        match (dir, from) {
            (Dir::Next, None) => rows.first().copied(),
            (Dir::Prev, None) => rows.last().copied(),
            (Dir::Next, Some(r)) => rows.get(rows.partition_point(|&x| x <= r)).copied(),
            (Dir::Prev, Some(r)) => {
                let at = rows.partition_point(|&x| x < r);
                at.checked_sub(1).and_then(|k| rows.get(k)).copied()
            }
        }
    }

    /// Moves the cursor to `row` and column `col` (else the cursor's column, else the first shown one); a column
    /// the view does not show is not landed on and never shown again (review F11).
    fn go_to_row(&mut self, row: u32, col: Option<u32>) {
        let col = col
            .filter(|&c| self.is_shown(c))
            .or_else(|| self.cursor().map(|c| c.col).filter(|&c| self.is_shown(c)))
            .or_else(|| self.shown.first().copied())
            .unwrap_or(0);
        let side = self.cursor().map_or(Side::Left, |c| c.side);
        self.set_cursor(Some(Cursor {
            row,
            col,
            side,
            whole_row: false,
        }));
        self.scroll = self.position_of(row);
        self.bump();
    }

    fn step_row(&mut self, dir: Dir) -> bool {
        let from = self.cursor().map(|c| c.row);
        match self.difference_row(from, dir) {
            Some(row) => {
                // The row's first differing cell that is shown (a column the view hides is skipped, F11).
                let col = self.sheet_cmp().and_then(|s| {
                    s.differing_cells(row)
                        .iter()
                        .filter(|d| !(self.iu() && d.state == CellState::Unimportant))
                        .find(|d| self.is_shown(d.column))
                        .map(|d| d.column)
                });
                self.go_to_row(row, col);
                true
            }
            None => false,
        }
    }

    /// The next (previous) differing cell of a shown row and a shown column (`nav::next_difference_cell`; rows the
    /// filter hides are jumped over with the visible differing rows, F12; columns the view hides are skipped, F11;
    /// an orphan row is one stop).
    fn step_cell(&mut self, dir: Dir) -> bool {
        let Some(sheet) = self.sheet_cmp() else {
            return false;
        };
        let rows = sheet.rows().len() as u32;
        let mut at = self.cursor().map(|c| (c.row, c.col));
        let target = loop {
            let Some((row, col)) = next_difference_cell(sheet, at, dir, self.iu()) else {
                break None;
            };
            if self.position_of(row).is_none() {
                // The filter hides the row: go on from the edge of the next visible differing row.
                let Some(next) = self.difference_row(Some(row), dir) else {
                    break None;
                };
                at = match dir {
                    Dir::Next => next.checked_sub(1).map(|r| (r, u32::MAX)),
                    Dir::Prev => Some(((next + 1).min(rows), 0)),
                };
                if dir == Dir::Prev && next + 1 >= rows {
                    at = None;
                }
                continue;
            }
            let orphan = sheet
                .rows()
                .get(row as usize)
                .is_some_and(|p| matches!(p.kind(), RowKind::LeftOnly | RowKind::RightOnly));
            if orphan {
                if self.cursor().is_some_and(|c| c.row == row) {
                    // The cursor is on this orphan row: the step leaves it.
                    at = Some((row, 0));
                    continue;
                }
                break Some((row, None));
            }
            if self.is_shown(col) {
                break Some((row, Some(col)));
            }
            at = Some((row, col));
        };
        match target {
            Some((row, col)) => {
                self.go_to_row(row, col);
                true
            }
            None => false,
        }
    }

    fn step_sheet(&mut self, dir: Dir) -> bool {
        let Some(cmp) = self.comparison() else {
            return false;
        };
        match next_difference_sheet(cmp, Some(self.sheet), dir, self.iu()) {
            Some(k) => {
                self.set_sheet(k);
                if self.cursor().is_none() {
                    self.step_row(Dir::Next);
                }
                true
            }
            None => false,
        }
    }

    /// The sheet pairs of the Go To dialog: their names.
    pub fn sheet_names(&self) -> Vec<String> {
        self.comparison().map_or_else(Vec::new, |c| {
            c.sheets().iter().map(|s| s.name().to_owned()).collect()
        })
    }

    /// Go To (TB-022, Ctrl+G): sheet `sheet` (else the active one), the row labelled `label` on either side (the
    /// left first), comparison column `col`; false when no shown row has that label.
    pub fn go_to(&mut self, sheet: Option<usize>, label: u64, col: Option<u32>) -> bool {
        if let Some(k) = sheet {
            self.set_sheet(k);
        }
        let Some(cmp) = self.response.as_ref().map(|r| Arc::clone(&r.comparison)) else {
            return false;
        };
        let found = [Side::Left, Side::Right].into_iter().find_map(|side| {
            self.visible()
                .iter()
                .copied()
                .find(|&row| cmp.row_label(self.sheet, row, side) == Some(label))
        });
        match found {
            Some(row) => {
                self.go_to_row(row, col);
                true
            }
            None => false,
        }
    }

    /// The comparison column whose name, or either side's letters, is `word` (case-insensitive): Go To's column.
    pub fn column_named(&self, word: &str) -> Option<u32> {
        let sheet = self.sheet_cmp()?;
        let word = word.trim();
        if word.is_empty() {
            return None;
        }
        (0..sheet.columns().len() as u32).find(|&c| {
            self.column_head(c).is_some_and(|h| {
                h.names
                    .iter()
                    .chain(h.letters.iter())
                    .flatten()
                    .any(|n| n.eq_ignore_ascii_case(word))
            })
        })
    }

    // ---- commands ----------------------------------------------------------------------------------------------

    /// Fills the command context of this view.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.busy = self.busy();
        ctx.ready = self.response.is_some();
        ctx.next_difference = self.nav.0;
        ctx.prev_difference = self.nav.1;
        ctx.parents_available = true;
    }

    fn set_filter(&mut self, filter: RowFilter) {
        self.filter = RowFilter {
            ignore_unimportant: self.filter.ignore_unimportant,
            ..filter
        };
        self.refilter();
    }

    fn preset_is(&self, preset: RowFilter) -> bool {
        RowFilter {
            ignore_unimportant: false,
            ..self.filter
        } == preset
    }

    /// The check state of a command (`None`: not a check or radio command of this view).
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        let f = self.filter;
        Some(match id {
            CommandId::FavCompareAll => self.preset_is(RowFilter::ALL),
            CommandId::FavDiff => self.preset_is(RowFilter::DIFFERENCES),
            CommandId::FavSame => self.preset_is(RowFilter::SAME),
            CommandId::FavCompareNone => self.preset_is(RowFilter::NONE),
            CommandId::TogSame => f.same,
            CommandId::TogLeftOrphan => f.left_only,
            CommandId::TogRightOrphan => f.right_only,
            CommandId::TogDiff => f.changed,
            CommandId::InsigIgnored => f.ignore_unimportant,
            CommandId::ShowThumbnail => self.show_thumbnail,
            CommandId::ShowDetails => self.show_details,
            CommandId::ColHideSame => self.hide_same,
            CommandId::SideBySide => self.layout == GridLayout::SideBySide,
            CommandId::OverUnder => self.layout == GridLayout::OverUnder,
            _ => return None,
        })
    }

    /// Runs a view command; true when it was this view's.
    pub fn execute(&mut self, service: &Service, id: CommandId) -> bool {
        match id {
            CommandId::Reload => self.start(service),
            CommandId::DiffItemNext => {
                self.step_row(Dir::Next);
            }
            CommandId::DiffItemPrev => {
                self.step_row(Dir::Prev);
            }
            CommandId::DiffNext => {
                self.step_cell(Dir::Next);
            }
            CommandId::DiffPrev => {
                self.step_cell(Dir::Prev);
            }
            CommandId::DiffSheetNext => {
                self.step_sheet(Dir::Next);
            }
            CommandId::DiffSheetPrev => {
                self.step_sheet(Dir::Prev);
            }
            CommandId::FavCompareAll => self.set_filter(RowFilter::ALL),
            CommandId::FavDiff => self.set_filter(RowFilter::DIFFERENCES),
            CommandId::FavSame => self.set_filter(RowFilter::SAME),
            CommandId::FavCompareNone => self.set_filter(RowFilter::NONE),
            CommandId::TogSame => {
                self.filter.same = !self.filter.same;
                self.refilter();
            }
            CommandId::TogLeftOrphan => {
                self.filter.left_only = !self.filter.left_only;
                self.refilter();
            }
            CommandId::TogRightOrphan => {
                self.filter.right_only = !self.filter.right_only;
                self.refilter();
            }
            CommandId::TogDiff => {
                self.filter.changed = !self.filter.changed;
                self.refilter();
            }
            CommandId::InsigIgnored => {
                self.filter.ignore_unimportant = !self.filter.ignore_unimportant;
                self.refilter();
            }
            CommandId::ColHideSame => {
                self.hide_same = !self.hide_same;
                self.rebuild_columns();
                self.bump();
            }
            CommandId::ColDetectSize => self.fit_columns(),
            CommandId::ShowThumbnail => {
                self.show_thumbnail = !self.show_thumbnail;
            }
            CommandId::ShowDetails => {
                self.show_details = !self.show_details;
            }
            CommandId::SideBySide => {
                self.layout = GridLayout::SideBySide;
            }
            CommandId::OverUnder => {
                self.layout = GridLayout::OverUnder;
            }
            CommandId::DisplayFontIncrease => self.set_font(self.font_px + 1),
            CommandId::DisplayFontDecrease => self.set_font(self.font_px.saturating_sub(1)),
            CommandId::DisplayFontReset => self.set_font(self.default_font_px),
            _ => return false,
        }
        true
    }

    fn set_font(&mut self, px: u32) {
        self.font_px = px.clamp(FONT_RANGE.0, FONT_RANGE.1);
    }

    /// Font size (px).
    pub fn font_px(&self) -> u32 {
        self.font_px
    }

    /// The thumbnail is shown.
    pub fn show_thumbnail(&self) -> bool {
        self.show_thumbnail
    }

    /// The Text Details panel is shown.
    pub fn show_details(&self) -> bool {
        self.show_details
    }

    /// Row numbers are shown (`[layout.table-compare] row_numbers`, TB-020's toggle).
    pub fn row_numbers(&self) -> bool {
        self.row_numbers
    }

    /// The layout of the two sides.
    pub fn layout(&self) -> GridLayout {
        self.layout
    }

    // ---- layout and session ------------------------------------------------------------------------------------

    /// Applies the layout of the view type and a session's `[view]` keys (D-108): `font_size` (else the theme's editor
    /// size), `thumbnail`, `details`, `row_numbers`, `layout`, `hide_same`; the session's `sheet`, `hidden_columns` and
    /// `widths` (T-53). A value of the wrong type is ignored.
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
            self.show_thumbnail = on;
        }
        if let Some(on) = flag("details") {
            self.show_details = on;
        }
        if let Some(on) = flag("row_numbers") {
            self.row_numbers = on;
        }
        if let Some(on) = flag("hide_same") {
            self.hide_same = on;
        }
        if let Some(LayoutValue::Text(word)) = value("layout") {
            self.layout = if word == GridLayout::OverUnder.word() {
                GridLayout::OverUnder
            } else {
                GridLayout::SideBySide
            };
        }
        if let Some(LayoutValue::Text(sheet)) = value("sheet") {
            self.wanted_sheet = Some(sheet);
        }
        if let Some(LayoutValue::Texts(list)) = value("hidden_columns") {
            self.hidden = list.iter().filter_map(|k| split_key(k)).collect();
        }
        if let Some(LayoutValue::Numbers(map)) = value("widths") {
            self.widths = map
                .iter()
                .filter_map(|(k, &w)| Some((split_key(k)?, w as f32)))
                .collect();
        }
        if self.response.is_some() {
            if let Some(name) = self.wanted_sheet.take()
                && let Some(k) = self.sheet_index(&name)
            {
                self.sheet = k;
            }
            self.activate_sheet();
        }
    }

    /// The layout keys of the view type (D-108; `[layout.table-compare]`): what every Table Compare view shares.
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("font_size", LayoutValue::Float(f64::from(self.font_px))),
            ("thumbnail", LayoutValue::Bool(self.show_thumbnail)),
            ("details", LayoutValue::Bool(self.show_details)),
            ("row_numbers", LayoutValue::Bool(self.row_numbers)),
            ("layout", LayoutValue::Text(self.layout.word().to_owned())),
            ("hide_same", LayoutValue::Bool(self.hide_same)),
        ]
    }

    /// The keys only this view's session keeps in `[view]` (T-53): the sheet shown, the hidden columns and the widths
    /// (`<sheet>/<column name>`; a sheet name never holds `/`).
    pub fn session_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        let mut keys = Vec::new();
        if let Some(name) = self
            .sheet_name(self.sheet)
            .or_else(|| self.wanted_sheet.clone())
        {
            keys.push(("sheet", LayoutValue::Text(name)));
        }
        if !self.hidden.is_empty() {
            keys.push((
                "hidden_columns",
                LayoutValue::Texts(
                    self.hidden
                        .iter()
                        .map(|(s, c)| format!("{s}/{c}"))
                        .collect(),
                ),
            ));
        }
        if !self.widths.is_empty() {
            keys.push((
                "widths",
                LayoutValue::Numbers(
                    self.widths
                        .iter()
                        .map(|((s, c), &w)| (format!("{s}/{c}"), f64::from(w)))
                        .collect(),
                ),
            ));
        }
        keys
    }

    /// The display word of the filter for the session's `[display] filter` (`all`, `differences`, `same`, `none`; a
    /// filter of toggles keeps the nearest preset's word: `session run` reads the presets only).
    pub fn display_word(&self) -> &'static str {
        let f = self.filter;
        let nearest = if f.preset_word().is_some() {
            f
        } else if f.same {
            RowFilter::ALL
        } else {
            RowFilter::DIFFERENCES
        };
        nearest.preset_word().unwrap_or_default()
    }

    /// The inputs watched for changes on disk (D-118).
    pub fn inputs(&self) -> [String; 2] {
        [lossy(&self.request.left), lossy(&self.request.right)]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, sessions).
    pub fn duplicate(&self) -> (ViewRequest, [Option<String>; 2]) {
        let mut request = self.request.clone();
        request.ignore_unimportant = self.filter.ignore_unimportant;
        request.display = Some(self.display_word().to_owned());
        (
            ViewRequest::Table {
                request,
                filter: self.filter,
            },
            self.titles.clone(),
        )
    }

    /// The running jobs (the job indicator).
    pub fn jobs(&self, view: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
        self.job
            .iter()
            .map(|job| {
                let (phase, fraction) = job.progress();
                JobStatus {
                    view,
                    title: job.title().to_owned(),
                    phase,
                    fraction,
                }
            })
            .collect()
    }

    /// The notes for the log since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    fn verdict(&self) -> Option<ContentKind> {
        let r = self.response.as_ref()?;
        Some(r.comparison.kind(r.bytes_identical, self.iu()))
    }

    fn summary_note(&self) -> Option<LogNote> {
        let r = self.response.as_ref()?;
        let stats = r.comparison.stats();
        Some(LogNote {
            kind: LogKind::Summary,
            text: format!(
                "{}: {} of {} sheets differ, {} important and {} unimportant rows, compared in {} ms.",
                verdict_text(self.verdict()?),
                stats.sheets_differing,
                r.comparison.sheets().len(),
                stats.totals.important_rows(),
                stats.totals.unimportant_rows(),
                self.ready_after.map_or(0, |d| d.as_millis()),
            ),
        })
    }

    /// The pane headers: per side the spec and what was read (`csv · 3 sheets`).
    pub fn pane_titles(&self) -> [(String, String); 2] {
        [0usize, 1].map(|k| {
            let spec = if k == 0 {
                &self.request.left
            } else {
                &self.request.right
            };
            let title =
                crate::shared::shown_path(&self.titles[k].clone().unwrap_or_else(|| lossy(spec)));
            let side = if k == 0 { Side::Left } else { Side::Right };
            let detail = self.comparison().map_or_else(String::new, |c| {
                let book = c.workbook(side);
                let sheet = c.sheet(self.sheet, side).map_or_else(
                    || "no such sheet".to_owned(),
                    |s| format!("{} rows", s.row_count()),
                );
                format!(
                    "{} · {} {} · {sheet}",
                    source_word(book.source()),
                    book.sheets().len(),
                    if book.sheets().len() == 1 {
                        "sheet"
                    } else {
                        "sheets"
                    }
                )
            });
            (title, detail)
        })
    }

    /// The status bar (TB-024): the address and state of the current cell and the sheet on the left; the verdict and
    /// the sheet's rows on the right.
    pub fn status_items(&self) -> StatusItems {
        let left = match (self.phase(), self.sheet_cmp()) {
            (TablePhase::Loading, _) => "Comparing the tables…".to_owned(),
            (TablePhase::Failed, _) if self.cancelled => "Cancelled.".to_owned(),
            (TablePhase::Failed, _) => "The tables could not be compared.".to_owned(),
            (TablePhase::Ready, None) => "No sheets.".to_owned(),
            (TablePhase::Ready, Some(sheet)) => {
                let mut parts = vec![format!("Sheet {}", sheet.name())];
                if let (Some(address), Some(cursor)) = (self.address(), self.cursor()) {
                    let state = match self.shown_state(sheet.cell_state(cursor.row, cursor.col)) {
                        CellState::Same => "same",
                        CellState::Unimportant => "unimportant difference",
                        CellState::Important => "important difference",
                    };
                    let side = if cursor.side == Side::Left {
                        "left"
                    } else {
                        "right"
                    };
                    parts.push(format!("{address} ({side}): {state}"));
                }
                parts.join(" · ")
            }
        };
        let mut right = Vec::new();
        if let (Some(kind), Some(sheet)) = (self.verdict(), self.sheet_cmp()) {
            let s = sheet.stats();
            right.push(verdict_text(kind).to_owned());
            right.push(format!(
                "{} important, {} unimportant, {} left only, {} right only of {} rows",
                s.changed_important,
                s.changed_unimportant,
                s.left_only_important + s.left_only_unimportant,
                s.right_only_important + s.right_only_unimportant,
                s.rows
            ));
            right.push(alignment_text(sheet.alignment()));
            right.push(format!(
                "compared in {} ms",
                self.ready_after.map_or(0, |d| d.as_millis())
            ));
        }
        StatusItems {
            left,
            right,
            ..StatusItems::default()
        }
    }

    /// The Info dialog's table (TB-024: the difference rows per sheet and the totals).
    pub fn info_sections(&self) -> Vec<crate::info_dialog::InfoSection> {
        use crate::info_dialog::{InfoRow, InfoSection};
        let Some(cmp) = self.comparison() else {
            return Vec::new();
        };
        let n = |v: u64| v.to_string();
        let block = |title: String, s: &hikasa_table::SheetStats, state: Option<SheetState>| {
            let row = |label: &str, values: Vec<String>| InfoRow {
                label: label.to_owned(),
                values,
            };
            let mut rows = Vec::new();
            if let Some(state) = state {
                rows.push(row(
                    "State",
                    vec![sheet_state_text(state).to_owned(), String::new()],
                ));
            }
            rows.extend([
                row("Rows", vec![n(s.left_rows), n(s.right_rows)]),
                row("Same rows", vec![n(s.same_rows), String::new()]),
                row(
                    "Changed rows (important / unimportant)",
                    vec![n(s.changed_important), n(s.changed_unimportant)],
                ),
                row(
                    "Orphan rows (left / right)",
                    vec![
                        n(s.left_only_important + s.left_only_unimportant),
                        n(s.right_only_important + s.right_only_unimportant),
                    ],
                ),
                row(
                    "Differing cells (important / unimportant)",
                    vec![n(s.cells_important), n(s.cells_unimportant)],
                ),
                row(
                    "Columns paired / one side (left / right)",
                    vec![
                        n(s.columns_paired),
                        format!("{} / {}", s.columns_left_only, s.columns_right_only),
                    ],
                ),
                row(
                    "Header differences",
                    vec![n(s.header_differences), String::new()],
                ),
                row(
                    "Rows with duplicate keys",
                    vec![n(s.duplicate_left), n(s.duplicate_right)],
                ),
            ]);
            InfoSection {
                title,
                heads: vec!["Left".to_owned(), "Right".to_owned()],
                rows,
            }
        };
        let stats = cmp.stats();
        let mut sections = vec![InfoSection {
            title: "Sheets".to_owned(),
            heads: Vec::new(),
            rows: vec![
                InfoRow {
                    label: "Paired".to_owned(),
                    values: vec![n(stats.sheets_paired)],
                },
                InfoRow {
                    label: "Left only".to_owned(),
                    values: vec![n(stats.sheets_left_only)],
                },
                InfoRow {
                    label: "Right only".to_owned(),
                    values: vec![n(stats.sheets_right_only)],
                },
                InfoRow {
                    label: "With differences".to_owned(),
                    values: vec![n(stats.sheets_differing)],
                },
            ],
        }];
        sections.push(block("All sheets".to_owned(), &stats.totals, None));
        for s in cmp.sheets() {
            sections.push(block(
                format!("Sheet {}", s.name()),
                s.stats(),
                Some(s.state()),
            ));
        }
        sections
    }

    /// The D-23 outcome when the view closes: the verdict under the view's ignore-unimportant (exit 1, 2, 12, 13),
    /// `LoadFailed` (100) when the tables could not be compared, `NoVerdict` while comparing or after a cancellation.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        self.poll(service);
        if let Some(r) = &self.response {
            let iu = self.iu();
            ViewOutcome::Text {
                result: r.source().verdict(iu),
                ignore_unimportant: iu,
            }
        } else if self.error.is_some() {
            ViewOutcome::LoadFailed
        } else {
            ViewOutcome::NoVerdict
        }
    }
}

impl Drop for TableViewModel {
    /// A view that closes or is replaced (a reopen) stops its comparison (review F3).
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The key of each column by its name (`hidden_columns`, `widths`): the name for its first occurrence, `name#2`,
/// `name#3`, … for the next ones, so two columns of the same name are two keys (review F10). The keys of earlier
/// versions (the bare name) read as the first occurrence.
fn column_keys<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    names
        .map(|name| {
            let n = seen.entry(name).or_insert(0);
            *n += 1;
            if *n == 1 {
                name.to_owned()
            } else {
                format!("{name}#{n}")
            }
        })
        .collect()
}

/// A field of a copied row: as it is, or quoted (`"…"`, a quote doubled) when it holds a tab, a line break or a
/// quote, as CSV/TSV quote it (review F5).
fn field(text: &str) -> String {
    if text.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Splits a `<sheet>/<column>` key of `[view]`.
fn split_key(key: &str) -> Option<(String, String)> {
    let (sheet, column) = key.split_once('/')?;
    Some((sheet.to_owned(), column.to_owned()))
}

/// A cell text cut at [`MAX_CELL_CHARS`], line breaks shown as `↵` (a cell is one line of the grid).
fn cut(text: &str) -> String {
    let mut out: String = text
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| match c {
            '\n' | '\r' => '↵',
            '\t' => ' ',
            c => c,
        })
        .collect();
    if text.chars().nth(MAX_CELL_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// Words of the verdict for the status bar.
pub fn verdict_text(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::BinarySame => "Identical",
        ContentKind::RulesSame => "No important differences",
        ContentKind::UnimportantOnly => "Only unimportant differences",
        ContentKind::ImportantDifferences => "Important differences",
        ContentKind::BinaryDifferent => "Binary differences",
        ContentKind::Error => "Error",
    }
}

/// Words of a sheet state (tab hints, Info).
pub fn sheet_state_text(state: SheetState) -> &'static str {
    match state {
        SheetState::Same => "same",
        SheetState::Unimportant => "unimportant differences",
        SheetState::Important => "important differences",
        SheetState::LeftOnly => "left only",
        SheetState::RightOnly => "right only",
    }
}

fn alignment_text(used: &RowAlignmentUsed) -> String {
    match used {
        RowAlignmentUsed::Keyed { keys } => format!(
            "rows by key ({} {})",
            keys.len(),
            if keys.len() == 1 { "column" } else { "columns" }
        ),
        RowAlignmentUsed::Engine { algorithm, sorted } => format!(
            "rows aligned ({}{})",
            format!("{algorithm:?}").to_lowercase(),
            if *sorted { ", sorted" } else { "" }
        ),
        RowAlignmentUsed::Orphan => "one side only".to_owned(),
    }
}

fn source_word(kind: hikasa_table::SourceKind) -> &'static str {
    use hikasa_table::SourceKind as K;
    match kind {
        K::Delimited => "delimited text",
        K::Fixed => "fixed-width text",
        K::Xlsx => "xlsx",
        K::Xlsm => "xlsm",
        K::Xlsb => "xlsb",
        K::Xls => "xls",
        K::Ods => "ods",
    }
}

fn display_name(spec: &std::ffi::OsStr) -> String {
    crate::shared::spec_name(&lossy(spec)).to_owned()
}

fn lossy(spec: &std::ffi::OsStr) -> String {
    spec.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_cells() {
        assert_eq!(
            split_key("Sheet 1/a/b"),
            Some(("Sheet 1".to_owned(), "a/b".to_owned()))
        );
        assert_eq!(split_key("nosheet"), None);
        assert_eq!(cut("a\nb\tc"), "a↵b c");
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a\tb"), "\"a\tb\"");
        assert_eq!(field("say \"hi\"\nnow"), "\"say \"\"hi\"\"\nnow\"");
        assert_eq!(
            column_keys(["x", "id", "x", "x"].into_iter()),
            ["x", "id", "x#2", "x#3"]
        );
        let long = "x".repeat(MAX_CELL_CHARS + 5);
        assert_eq!(cut(&long).chars().count(), MAX_CELL_CHARS + 1);
    }
}
