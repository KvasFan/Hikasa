//! The result model (contract §5.5): [`TableComparison`], [`SheetComparison`], [`RowPair`], [`CellDiff`],
//! [`ColumnPair`], statistics, filters.

use std::sync::Arc;

use hikasa_core::{ContentKind, Side};
use hikasa_diff::Algorithm;
use serde::{Deserialize, Serialize};

use crate::cell::Value;
use crate::compare::options::ColumnRule;
use crate::error::TableWarning;
use crate::sheet::{ColumnKind, Sheet, Workbook};

/// State of one cell pair (§5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    /// Same.
    Same,
    /// An unimportant difference.
    Unimportant,
    /// An important difference.
    Important,
}

impl CellState {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            CellState::Same => "same",
            CellState::Unimportant => "unimportant",
            CellState::Important => "important",
        }
    }
}

/// Kind of a row of the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// Paired, every cell same.
    Same,
    /// Paired, with differences.
    Changed,
    /// Left only.
    LeftOnly,
    /// Right only.
    RightOnly,
}

impl RowKind {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            RowKind::Same => "same",
            RowKind::Changed => "changed",
            RowKind::LeftOnly => "left_only",
            RowKind::RightOnly => "right_only",
        }
    }
}

/// Importance of a row's differences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    /// No difference.
    None,
    /// Only unimportant differences.
    Unimportant,
    /// At least one important difference.
    Important,
}

impl Importance {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Importance::None => "none",
            Importance::Unimportant => "unimportant",
            Importance::Important => "important",
        }
    }

    pub(crate) fn of(state: CellState) -> Importance {
        match state {
            CellState::Same => Importance::None,
            CellState::Unimportant => Importance::Unimportant,
            CellState::Important => Importance::Important,
        }
    }
}

const NONE: u32 = u32::MAX;

/// One row of a sheet comparison: 12 bytes (data rows of the two sheets, header excluded; see
/// [`TableComparison::cell`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowPair {
    left: u32,
    right: u32,
    kind: RowKind,
    importance: Importance,
}

impl RowPair {
    pub(crate) fn new(
        left: Option<u32>,
        right: Option<u32>,
        kind: RowKind,
        importance: Importance,
    ) -> RowPair {
        RowPair {
            left: left.unwrap_or(NONE),
            right: right.unwrap_or(NONE),
            kind,
            importance,
        }
    }

    /// Left data row.
    pub fn left(&self) -> Option<u32> {
        (self.left != NONE).then_some(self.left)
    }

    /// Right data row.
    pub fn right(&self) -> Option<u32> {
        (self.right != NONE).then_some(self.right)
    }

    /// Data row of a side.
    pub fn side(&self, side: Side) -> Option<u32> {
        match side {
            Side::Left => self.left(),
            _ => self.right(),
        }
    }

    /// Kind.
    pub fn kind(&self) -> RowKind {
        self.kind
    }

    /// Importance.
    pub fn importance(&self) -> Importance {
        self.importance
    }

    /// True when the row is a difference (important, or unimportant when not ignored).
    pub fn differs(&self, ignore_unimportant: bool) -> bool {
        match self.importance {
            Importance::None => false,
            Importance::Unimportant => !ignore_unimportant,
            Importance::Important => true,
        }
    }
}

/// A differing cell of a paired row (sparse; sorted by row, then column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellDiff {
    /// Index of the row in [`SheetComparison::rows`].
    pub row: u32,
    /// Comparison column.
    pub column: u32,
    /// State (never `Same`).
    pub state: CellState,
}

/// One comparison column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnPair {
    /// Left column (0-based in the left sheet).
    pub left: Option<u32>,
    /// Right column.
    pub right: Option<u32>,
    /// Display name (the left header name, else the right, else letters).
    pub name: String,
    /// The effective rule.
    pub rule: ColumnRule,
    /// Key priority (0 = first key).
    pub key: Option<u8>,
    /// Hidden (display only).
    pub hidden: bool,
    /// Detected kind of the left column.
    pub left_kind: ColumnKind,
    /// Detected kind of the right column.
    pub right_kind: ColumnKind,
}

/// Duplicate keys of a keyed join (T-8).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DuplicateKeys {
    /// Left rows whose key occurs more than once on the left.
    pub left_rows: u64,
    /// Right rows likewise.
    pub right_rows: u64,
    /// Up to 10 left examples: the key's display texts and the row labels.
    pub left_examples: Vec<(Vec<String>, Vec<u64>)>,
    /// Up to 10 right examples.
    pub right_examples: Vec<(Vec<String>, Vec<u64>)>,
}

/// How the rows were aligned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum RowAlignmentUsed {
    /// By key columns (comparison column indices, priority order).
    Keyed {
        /// Key columns.
        keys: Vec<u32>,
    },
    /// Through `hikasa_diff::align`.
    Engine {
        /// Engine.
        algorithm: Algorithm,
        /// Rows were sorted first.
        sorted: bool,
    },
    /// A sheet on one side only.
    Orphan,
}

/// State of a sheet pair (the TB-004 tab spot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetState {
    /// No difference.
    Same,
    /// Only unimportant differences.
    Unimportant,
    /// Important differences.
    Important,
    /// Left only.
    LeftOnly,
    /// Right only.
    RightOnly,
}

impl SheetState {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SheetState::Same => "same",
            SheetState::Unimportant => "unimportant",
            SheetState::Important => "important",
            SheetState::LeftOnly => "left_only",
            SheetState::RightOnly => "right_only",
        }
    }
}

/// Statistics of one sheet pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SheetStats {
    /// Data rows of the left sheet.
    pub left_rows: u64,
    /// Data rows of the right sheet.
    pub right_rows: u64,
    /// Rows of the comparison.
    pub rows: u64,
    /// Same rows.
    pub same_rows: u64,
    /// Changed rows with an important difference.
    pub changed_important: u64,
    /// Changed rows with only unimportant differences.
    pub changed_unimportant: u64,
    /// Important left-only rows.
    pub left_only_important: u64,
    /// Unimportant left-only rows.
    pub left_only_unimportant: u64,
    /// Important right-only rows.
    pub right_only_important: u64,
    /// Unimportant right-only rows.
    pub right_only_unimportant: u64,
    /// Cells compared (paired rows × comparison columns).
    pub cells_compared: u64,
    /// Same cells.
    pub cells_same: u64,
    /// Unimportant cells.
    pub cells_unimportant: u64,
    /// Important cells.
    pub cells_important: u64,
    /// Paired columns.
    pub columns_paired: u64,
    /// Left-only columns.
    pub columns_left_only: u64,
    /// Right-only columns.
    pub columns_right_only: u64,
    /// Differing header names (important or unimportant).
    pub header_differences: u64,
    /// Rows with duplicate keys, left.
    pub duplicate_left: u64,
    /// Rows with duplicate keys, right.
    pub duplicate_right: u64,
}

impl SheetStats {
    pub(crate) fn add(&mut self, o: &SheetStats) {
        self.left_rows += o.left_rows;
        self.right_rows += o.right_rows;
        self.rows += o.rows;
        self.same_rows += o.same_rows;
        self.changed_important += o.changed_important;
        self.changed_unimportant += o.changed_unimportant;
        self.left_only_important += o.left_only_important;
        self.left_only_unimportant += o.left_only_unimportant;
        self.right_only_important += o.right_only_important;
        self.right_only_unimportant += o.right_only_unimportant;
        self.cells_compared += o.cells_compared;
        self.cells_same += o.cells_same;
        self.cells_unimportant += o.cells_unimportant;
        self.cells_important += o.cells_important;
        self.columns_paired += o.columns_paired;
        self.columns_left_only += o.columns_left_only;
        self.columns_right_only += o.columns_right_only;
        self.header_differences += o.header_differences;
        self.duplicate_left += o.duplicate_left;
        self.duplicate_right += o.duplicate_right;
    }

    /// Rows that differ importantly.
    pub fn important_rows(&self) -> u64 {
        self.changed_important + self.left_only_important + self.right_only_important
    }

    /// Rows with only unimportant differences.
    pub fn unimportant_rows(&self) -> u64 {
        self.changed_unimportant + self.left_only_unimportant + self.right_only_unimportant
    }
}

/// Statistics of a comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TableStats {
    /// Totals over the sheet pairs.
    pub totals: SheetStats,
    /// Paired sheets.
    pub sheets_paired: u64,
    /// Left-only sheets.
    pub sheets_left_only: u64,
    /// Right-only sheets.
    pub sheets_right_only: u64,
    /// Sheet pairs whose state is not `Same` (orphans included).
    pub sheets_differing: u64,
}

/// One side of a sheet pair as compared: the sheet and whether its first stored row was the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SideRef {
    pub sheet: u32,
    pub first: u32,
}

/// The comparison of one sheet pair.
#[derive(Debug, Clone)]
pub struct SheetComparison {
    pub(crate) left: Option<SideRef>,
    pub(crate) right: Option<SideRef>,
    pub(crate) name: String,
    pub(crate) renamed: bool,
    pub(crate) columns: Vec<ColumnPair>,
    pub(crate) rows: Vec<RowPair>,
    pub(crate) diffs: Vec<CellDiff>,
    pub(crate) header: Vec<CellState>,
    pub(crate) stats: SheetStats,
    pub(crate) state: SheetState,
    pub(crate) alignment: RowAlignmentUsed,
    pub(crate) duplicates: DuplicateKeys,
}

impl SheetComparison {
    /// Left sheet index.
    pub fn left(&self) -> Option<u32> {
        self.left.map(|s| s.sheet)
    }

    /// Right sheet index.
    pub fn right(&self) -> Option<u32> {
        self.right.map(|s| s.sheet)
    }

    /// Name (the left sheet's, else the right's).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The paired sheets have different names (informational, T-10).
    pub fn renamed(&self) -> bool {
        self.renamed
    }

    /// Comparison columns.
    pub fn columns(&self) -> &[ColumnPair] {
        &self.columns
    }

    /// Rows in comparison order.
    pub fn rows(&self) -> &[RowPair] {
        &self.rows
    }

    /// Differing cells of row `row` (sorted by column).
    pub fn differing_cells(&self, row: u32) -> &[CellDiff] {
        let a = self.diffs.partition_point(|d| d.row < row);
        let b = self.diffs.partition_point(|d| d.row <= row);
        &self.diffs[a..b]
    }

    /// Every differing cell.
    pub fn all_differing_cells(&self) -> &[CellDiff] {
        &self.diffs
    }

    /// State of a cell of row `row`, column `column` (orphan rows: their importance).
    pub fn cell_state(&self, row: u32, column: u32) -> CellState {
        let Some(r) = self.rows.get(row as usize) else {
            return CellState::Same;
        };
        match r.kind {
            RowKind::Same => CellState::Same,
            RowKind::Changed => self
                .differing_cells(row)
                .iter()
                .find(|d| d.column == column)
                .map_or(CellState::Same, |d| d.state),
            RowKind::LeftOnly | RowKind::RightOnly => match r.importance {
                Importance::Important => CellState::Important,
                Importance::Unimportant => CellState::Unimportant,
                Importance::None => CellState::Same,
            },
        }
    }

    /// Header name states per column (empty when a side has no header).
    pub fn header_states(&self) -> &[CellState] {
        &self.header
    }

    /// Statistics.
    pub fn stats(&self) -> &SheetStats {
        &self.stats
    }

    /// State (the sheet tab's spot).
    pub fn state(&self) -> SheetState {
        self.state
    }

    /// How the rows were aligned.
    pub fn alignment(&self) -> &RowAlignmentUsed {
        &self.alignment
    }

    /// Duplicate keys.
    pub fn duplicates(&self) -> &DuplicateKeys {
        &self.duplicates
    }

    /// True when the first stored row of `side`'s sheet was the header in this comparison.
    pub fn has_header(&self, side: Side) -> bool {
        self.side(side).is_some_and(|s| s.first == 1)
    }

    pub(crate) fn side(&self, side: Side) -> Option<SideRef> {
        match side {
            Side::Left => self.left,
            _ => self.right,
        }
    }

    /// Rows that pass `filter`.
    pub fn visible<'a>(&'a self, filter: &'a RowFilter) -> impl Iterator<Item = u32> + 'a {
        self.rows
            .iter()
            .enumerate()
            .filter(move |(_, r)| filter.shows(r))
            .map(|(i, _)| i as u32)
    }
}

/// Display filter of rows (TB-021).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RowFilter {
    /// Show same rows.
    pub same: bool,
    /// Show changed rows.
    pub changed: bool,
    /// Show left-only rows.
    pub left_only: bool,
    /// Show right-only rows.
    pub right_only: bool,
    /// Rows with only unimportant differences count as same.
    pub ignore_unimportant: bool,
}

impl RowFilter {
    /// Show All.
    pub const ALL: RowFilter = RowFilter {
        same: true,
        changed: true,
        left_only: true,
        right_only: true,
        ignore_unimportant: false,
    };
    /// Show Differences.
    pub const DIFFERENCES: RowFilter = RowFilter {
        same: false,
        ..RowFilter::ALL
    };
    /// Show Same.
    pub const SAME: RowFilter = RowFilter {
        same: true,
        changed: false,
        left_only: false,
        right_only: false,
        ignore_unimportant: false,
    };
    /// Show None.
    pub const NONE: RowFilter = RowFilter {
        same: false,
        changed: false,
        left_only: false,
        right_only: false,
        ignore_unimportant: false,
    };

    /// Parses `all`, `differences` (`mismatches`), `same` (`matches`), `none`.
    pub fn parse(text: &str) -> Result<RowFilter, String> {
        match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "all" => Ok(RowFilter::ALL),
            "differences" | "mismatches" | "diff" => Ok(RowFilter::DIFFERENCES),
            "same" | "matches" => Ok(RowFilter::SAME),
            "none" => Ok(RowFilter::NONE),
            _ => Err(format!(
                "unknown display `{text}` (expected all, differences, same or none)"
            )),
        }
    }

    /// True when `row` is shown.
    pub fn shows(&self, row: &RowPair) -> bool {
        let differs = row.differs(self.ignore_unimportant);
        match row.kind {
            _ if !differs => self.same,
            RowKind::Same | RowKind::Changed => self.changed,
            RowKind::LeftOnly => self.left_only,
            RowKind::RightOnly => self.right_only,
        }
    }
}

/// The result of a comparison: immutable, `Send + Sync`.
#[derive(Debug, Clone)]
pub struct TableComparison {
    pub(crate) left: Arc<Workbook>,
    pub(crate) right: Arc<Workbook>,
    pub(crate) sheets: Vec<SheetComparison>,
    pub(crate) stats: TableStats,
    pub(crate) warnings: Vec<TableWarning>,
}

impl TableComparison {
    /// Sheet pairs in comparison order.
    pub fn sheets(&self) -> &[SheetComparison] {
        &self.sheets
    }

    /// Statistics.
    pub fn stats(&self) -> &TableStats {
        &self.stats
    }

    /// Warnings of the loads and of the comparison.
    pub fn warnings(&self) -> &[TableWarning] {
        &self.warnings
    }

    /// The left workbook.
    pub fn left(&self) -> &Arc<Workbook> {
        &self.left
    }

    /// The right workbook.
    pub fn right(&self) -> &Arc<Workbook> {
        &self.right
    }

    /// The workbook of a side.
    pub fn workbook(&self, side: Side) -> &Workbook {
        match side {
            Side::Left => &self.left,
            _ => &self.right,
        }
    }

    /// The sheet of `side` in sheet pair `pair`.
    pub fn sheet(&self, pair: usize, side: Side) -> Option<&Sheet> {
        let s = self.sheets.get(pair)?.side(side)?;
        self.workbook(side).sheets.get(s.sheet as usize)
    }

    /// Value of `side`'s cell in row `row` (an index of [`SheetComparison::rows`]) and comparison column
    /// `column` of sheet pair `pair` (`Empty` when the side has no such row or column).
    pub fn cell(&self, pair: usize, row: u32, column: u32, side: Side) -> Value<'_> {
        let Some(cmp) = self.sheets.get(pair) else {
            return Value::Empty;
        };
        let (Some(r), Some(col), Some(s)) = (
            cmp.rows.get(row as usize).and_then(|r| r.side(side)),
            cmp.columns.get(column as usize).and_then(|c| match side {
                Side::Left => c.left,
                _ => c.right,
            }),
            cmp.side(side),
        ) else {
            return Value::Empty;
        };
        self.workbook(side)
            .sheets
            .get(s.sheet as usize)
            .map_or(Value::Empty, |sheet| sheet.stored_row(r + s.first).get(col))
    }

    /// Formula of a cell (same addressing as [`TableComparison::cell`]).
    pub fn formula(&self, pair: usize, row: u32, column: u32, side: Side) -> Option<&str> {
        let cmp = self.sheets.get(pair)?;
        let r = cmp.rows.get(row as usize)?.side(side)?;
        let c = cmp.columns.get(column as usize)?;
        let col = match side {
            Side::Left => c.left,
            _ => c.right,
        }?;
        let s = cmp.side(side)?;
        self.workbook(side)
            .sheets
            .get(s.sheet as usize)?
            .stored_formula(r + s.first, col)
    }

    /// 1-based label of `side`'s row (the text line, or the sheet row number).
    pub fn row_label(&self, pair: usize, row: u32, side: Side) -> Option<u64> {
        let cmp = self.sheets.get(pair)?;
        let r = cmp.rows.get(row as usize)?.side(side)?;
        let s = cmp.side(side)?;
        Some(
            self.workbook(side)
                .sheets
                .get(s.sheet as usize)?
                .stored_label(r + s.first),
        )
    }

    /// The verdict (T-16): `binary_same` when the caller found the bytes identical, else rules-same,
    /// unimportant-only or important differences.
    pub fn kind(&self, bytes_identical: bool, ignore_unimportant: bool) -> ContentKind {
        if bytes_identical {
            return ContentKind::BinarySame;
        }
        let mut unimportant = false;
        for s in &self.sheets {
            match s.state {
                SheetState::Important => return ContentKind::ImportantDifferences,
                SheetState::LeftOnly | SheetState::RightOnly => {
                    if s.is_empty_orphan() {
                        unimportant = true;
                    } else {
                        return ContentKind::ImportantDifferences;
                    }
                }
                SheetState::Unimportant => unimportant = true,
                SheetState::Same => {}
            }
        }
        if unimportant && !ignore_unimportant {
            ContentKind::UnimportantOnly
        } else {
            ContentKind::RulesSame
        }
    }
}

impl SheetComparison {
    /// True for a one-sided sheet without an important row (empty, or blank rows only): an unimportant difference
    /// (T-10).
    pub fn is_empty_orphan(&self) -> bool {
        self.rows
            .iter()
            .all(|r| r.importance != Importance::Important)
    }
}
