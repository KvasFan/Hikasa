//! `hikasa.table_compare.v1` (Table Compare contract §6.3, T-20): [`table_compare_json`] builds the document from the
//! comparison, the caller's sides and the view (row filter, cells). Arrays are never `null`; counts are numbers; cell
//! values are always display texts (§4.1), so the file's own spelling is kept.

use hikasa_core::{ContentResult, Side};
use hikasa_table::{
    CellState, ColumnPair, ColumnRule, DuplicateKeys, Importance, RowAlignmentUsed, RowFilter,
    RowKind, SheetAlignment, SheetComparison, SheetStats, TableKind, TableWarning, Value,
};
use serde::Serialize;

use super::FormatJson;
use crate::table::{TableCells, TableSide, TableSource, display_word, selector_text, sheets_word};

/// The view of the JSON document: the rows it carries and their cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableViewOptions {
    /// The rows shown (its `ignore_unimportant` is replaced by [`TableViewOptions::ignore_unimportant`]).
    pub filter: RowFilter,
    /// The cells of each row.
    pub cells: TableCells,
    /// Unimportant differences count as same (rows shown, the verdict).
    pub ignore_unimportant: bool,
}

impl Default for TableViewOptions {
    /// The differing rows with their differing cells, ignore-unimportant off.
    fn default() -> TableViewOptions {
        TableViewOptions {
            filter: RowFilter::DIFFERENCES,
            cells: TableCells::Differing,
            ignore_unimportant: false,
        }
    }
}

/// One side: `{spec, size, mtime, source, sheets, encoding, format}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableSideJson {
    /// The canonical spec.
    pub spec: String,
    /// Size in bytes.
    pub size: Option<u64>,
    /// Modification time, RFC 3339 UTC.
    pub mtime: Option<String>,
    /// `delimited`, `fixed`, `xlsx`, `xlsm`, `xlsb`, `xls`, `ods`.
    pub source: &'static str,
    /// Sheets of the workbook (1 for text tables).
    pub sheets: u64,
    /// Encoding of a text table (`null` for workbooks).
    pub encoding: Option<String>,
    /// The Hikasa file format chosen for the side (an addition to §6.3), `null` when none was.
    pub format: Option<FormatJson>,
}

/// Regional settings: `{decimal, thousands, date_order, date_sep}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegionalJson {
    /// Decimal separator.
    pub decimal: String,
    /// Thousands separator (`null`: none).
    pub thousands: Option<String>,
    /// `ymd`, `dmy`, `mdy`.
    pub date_order: &'static str,
    /// Date separator (`null`: any of `-`, `/`, `.`).
    pub date_sep: Option<String>,
}

/// The effective table format of a side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableFormatJson {
    /// `delimited`, `fixed`, `spreadsheet`.
    pub kind: &'static str,
    /// Delimiters (delimited tables; else empty).
    pub delimiters: Vec<String>,
    /// Text qualifier (`null`: none, or not delimited).
    pub qualifier: Option<String>,
    /// Consecutive delimiters count as one.
    pub consecutive_as_one: bool,
    /// Whitespace around fields is part of the delimiter.
    pub trim_whitespace: bool,
    /// Fixed widths (fixed tables; else empty).
    pub widths: Vec<u32>,
    /// `detect`, `names`, `data`.
    pub first_line: &'static str,
    /// Regional settings.
    pub regional: RegionalJson,
    /// Encoding (the decoded one of a text table; `null` for workbooks).
    pub encoding: Option<String>,
    /// The type was detected from the content (the format asked for *Detect*).
    pub detected: bool,
}

/// `{left, right}` of the formats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableFormatsJson {
    /// Left.
    pub left: TableFormatJson,
    /// Right.
    pub right: TableFormatJson,
}

/// One explicit sheet pair: `{left, right}` as the vocabulary writes them (`3`, `Data`, `name:3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetPairJson {
    /// Left sheet.
    pub left: Option<String>,
    /// Right sheet.
    pub right: Option<String>,
}

/// The sheet alignment: `{mode, pairs}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetsOptionJson {
    /// `by_name`, `left_name`, `right_name`, `position`, `custom`.
    pub mode: &'static str,
    /// The pairs of `custom` (else empty).
    pub pairs: Vec<SheetPairJson>,
}

/// The options: `{sheets, compare_formulas}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableOptionsJson {
    /// Sheet alignment.
    pub sheets: SheetsOptionJson,
    /// Formula texts are compared too.
    pub compare_formulas: bool,
}

/// Statistics of a sheet pair (or the totals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SheetStatsJson {
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
    /// Changed rows with unimportant differences only.
    pub changed_unimportant: u64,
    /// Important left-only rows.
    pub left_only_important: u64,
    /// Unimportant left-only rows.
    pub left_only_unimportant: u64,
    /// Important right-only rows.
    pub right_only_important: u64,
    /// Unimportant right-only rows.
    pub right_only_unimportant: u64,
    /// Cells compared.
    pub cells_compared: u64,
    /// Same cells.
    pub cells_same: u64,
    /// Cells with an unimportant difference.
    pub cells_unimportant: u64,
    /// Cells with an important difference.
    pub cells_important: u64,
    /// Columns on both sides.
    pub columns_paired: u64,
    /// Left-only columns.
    pub columns_left_only: u64,
    /// Right-only columns.
    pub columns_right_only: u64,
    /// Differing header names.
    pub header_differences: u64,
    /// Left rows whose key is not unique.
    pub duplicate_key_rows_left: u64,
    /// Right rows whose key is not unique.
    pub duplicate_key_rows_right: u64,
}

impl SheetStatsJson {
    fn new(s: &SheetStats) -> SheetStatsJson {
        SheetStatsJson {
            left_rows: s.left_rows,
            right_rows: s.right_rows,
            rows: s.rows,
            same_rows: s.same_rows,
            changed_important: s.changed_important,
            changed_unimportant: s.changed_unimportant,
            left_only_important: s.left_only_important,
            left_only_unimportant: s.left_only_unimportant,
            right_only_important: s.right_only_important,
            right_only_unimportant: s.right_only_unimportant,
            cells_compared: s.cells_compared,
            cells_same: s.cells_same,
            cells_unimportant: s.cells_unimportant,
            cells_important: s.cells_important,
            columns_paired: s.columns_paired,
            columns_left_only: s.columns_left_only,
            columns_right_only: s.columns_right_only,
            header_differences: s.header_differences,
            duplicate_key_rows_left: s.duplicate_left,
            duplicate_key_rows_right: s.duplicate_right,
        }
    }
}

/// Statistics of the comparison: the sheet counts and the totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TableStatsJson {
    /// Paired sheets.
    pub sheets_paired: u64,
    /// Left-only sheets.
    pub sheets_left_only: u64,
    /// Right-only sheets.
    pub sheets_right_only: u64,
    /// Sheet pairs that are not same (orphans included).
    pub sheets_differing: u64,
    /// The totals over the sheet pairs.
    pub totals: SheetStatsJson,
}

/// A sheet of a side: `{index, name}` (1-based index).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetRefJson {
    /// 1-based position in its workbook.
    pub index: u32,
    /// Name.
    pub name: String,
}

/// How the rows were aligned: `{mode, keys, algorithm, sorted}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlignmentUsedJson {
    /// `keyed`, `engine`, `orphan` (a sheet on one side only).
    pub mode: &'static str,
    /// The key columns' names, in priority order (`keyed`; else empty).
    pub keys: Vec<String>,
    /// The engine (`engine`; else `null`).
    pub algorithm: Option<&'static str>,
    /// The rows were sorted first (`engine`).
    pub sorted: bool,
}

/// A column of a side: `{index, letter, name}` (1-based index in the side's sheet; the header name or `null`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnSideJson {
    /// 1-based column of the sheet.
    pub index: u32,
    /// The sheet's own letters.
    pub letter: String,
    /// The header name (`null` when the comparison read no header on that side).
    pub name: Option<String>,
}

/// The effective rule of a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnRuleJson {
    /// `general`, `text`, `number`.
    pub kind: &'static str,
    /// Differences in the column are unimportant.
    pub unimportant: bool,
    /// Case differences are unimportant.
    pub ignore_case: bool,
    /// Whitespace differences are unimportant.
    pub ignore_whitespace: bool,
    /// Numeric tolerance (`0.01`, `0.5%`), `null` for none.
    pub numeric_tolerance: Option<String>,
    /// Date tolerance in milliseconds, `null` for none.
    pub date_tolerance_ms: Option<i64>,
    /// Workbook numbers compare by their binary value.
    pub exact_numbers: bool,
}

impl ColumnRuleJson {
    fn new(r: &ColumnRule) -> ColumnRuleJson {
        ColumnRuleJson {
            kind: r.kind.as_str(),
            unimportant: r.unimportant,
            ignore_case: r.ignore_case,
            ignore_whitespace: r.ignore_whitespace,
            numeric_tolerance: r.numeric_tolerance.map(|t| t.to_string()),
            date_tolerance_ms: r.date_tolerance_ms,
            exact_numbers: r.exact_numbers,
        }
    }
}

/// A comparison column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableColumnJson {
    /// 1-based comparison column.
    pub index: u32,
    /// Display name.
    pub name: String,
    /// The left column (`null`: right only).
    pub left: Option<ColumnSideJson>,
    /// The right column (`null`: left only).
    pub right: Option<ColumnSideJson>,
    /// Key priority (1 = first key), `null` for no key.
    pub key: Option<u32>,
    /// The effective rule.
    pub rule: ColumnRuleJson,
    /// Detected kind of the left column (`null` when absent).
    pub kind_left: Option<&'static str>,
    /// Detected kind of the right column.
    pub kind_right: Option<&'static str>,
    /// Hidden (display only).
    pub hidden: bool,
}

/// One duplicated key: its display texts and its row labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateExampleJson {
    /// The key's cells.
    pub key: Vec<String>,
    /// Row labels of the rows with that key.
    pub rows: Vec<u64>,
}

/// Duplicate keys of a keyed join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicatesJson {
    /// Left rows whose key is not unique.
    pub left_rows: u64,
    /// Right rows likewise.
    pub right_rows: u64,
    /// Up to 10 left examples.
    pub left_examples: Vec<DuplicateExampleJson>,
    /// Up to 10 right examples.
    pub right_examples: Vec<DuplicateExampleJson>,
}

impl DuplicatesJson {
    fn new(d: &DuplicateKeys) -> DuplicatesJson {
        let examples = |e: &[(Vec<String>, Vec<u64>)]| {
            e.iter()
                .map(|(key, rows)| DuplicateExampleJson {
                    key: key.clone(),
                    rows: rows.clone(),
                })
                .collect()
        };
        DuplicatesJson {
            left_rows: d.left_rows,
            right_rows: d.right_rows,
            left_examples: examples(&d.left_examples),
            right_examples: examples(&d.right_examples),
        }
    }
}

/// A header cell: `{column, state, left, right}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeaderCellJson {
    /// 1-based comparison column.
    pub column: u32,
    /// `same`, `unimportant`, `important`.
    pub state: &'static str,
    /// The left header name (`null`: no left column).
    pub left: Option<String>,
    /// The right header name.
    pub right: Option<String>,
}

/// The toggles of the row filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TogglesJson {
    /// Same rows.
    pub same: bool,
    /// Changed rows.
    pub changed: bool,
    /// Left-only rows.
    pub left_only: bool,
    /// Right-only rows.
    pub right_only: bool,
}

/// The display of a sheet's rows: `{filter, toggles, cells}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableDisplayJson {
    /// `all`, `differences`, `same`, `none`, `custom`.
    pub filter: &'static str,
    /// The toggles.
    pub toggles: TogglesJson,
    /// `differing`, `all`.
    pub cells: &'static str,
}

/// A cell value: `{value, type, formula}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellJson {
    /// The display text (§4.1).
    pub value: String,
    /// `number`, `text`, `date_time`, `duration`, `bool`, `error`, `empty`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// The formula text, `null` for none.
    pub formula: Option<String>,
}

/// A cell pair of a row: `{column, state, left, right}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellPairJson {
    /// 1-based comparison column.
    pub column: u32,
    /// `same`, `unimportant`, `important`.
    pub state: &'static str,
    /// The left cell (`null`: no left row or column).
    pub left: Option<CellJson>,
    /// The right cell.
    pub right: Option<CellJson>,
}

/// A row of the comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableRowJson {
    /// 1-based row of the comparison.
    pub row: u32,
    /// `same`, `changed`, `left_only`, `right_only`.
    pub state: &'static str,
    /// `none`, `unimportant`, `important`.
    pub importance: &'static str,
    /// The left row's label (its line, or its sheet row), `null` for none.
    pub left: Option<u64>,
    /// The right row's label.
    pub right: Option<u64>,
    /// Its cells (per `display.cells`).
    pub cells: Vec<CellPairJson>,
}

/// One sheet pair.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableSheetJson {
    /// Name (the left sheet's, else the right's).
    pub name: String,
    /// The left sheet, `null` when right only.
    pub left: Option<SheetRefJson>,
    /// The right sheet.
    pub right: Option<SheetRefJson>,
    /// The paired sheets' names differ (informational).
    pub renamed: bool,
    /// `same`, `unimportant`, `important`, `left_only`, `right_only`.
    pub state: &'static str,
    /// How the rows were aligned.
    pub alignment: AlignmentUsedJson,
    /// The comparison columns.
    pub columns: Vec<TableColumnJson>,
    /// Statistics.
    pub stats: SheetStatsJson,
    /// Duplicate keys.
    pub duplicates: DuplicatesJson,
    /// The header names per column with their states, `null` when a side has no header.
    pub header: Option<Vec<HeaderCellJson>>,
    /// The display.
    pub display: TableDisplayJson,
    /// The shown rows.
    pub rows: Vec<TableRowJson>,
    /// Rows the filter hides.
    pub hidden_rows: u64,
}

/// A warning: `{code, message, side, sheet}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableWarningJson {
    /// Stable code.
    pub code: &'static str,
    /// Message.
    pub message: String,
    /// `left`, `right`, `null`.
    pub side: Option<&'static str>,
    /// The sheet, `null` for none.
    pub sheet: Option<String>,
}

/// `hikasa.table_compare.v1` (§6.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableCompareJson {
    /// Left side.
    pub left: TableSideJson,
    /// Right side.
    pub right: TableSideJson,
    /// The effective table formats.
    pub format: TableFormatsJson,
    /// The comparison options.
    pub options: TableOptionsJson,
    /// Unimportant differences count as same.
    pub ignore_unimportant: bool,
    /// The verdict (`{"result": …}`).
    pub verdict: ContentResult,
    /// Statistics.
    pub stats: TableStatsJson,
    /// Sheet pairs in comparison order.
    pub sheets: Vec<TableSheetJson>,
    /// The table warnings (loads and comparison).
    pub warnings: Vec<TableWarningJson>,
}

/// The JSON report (`--layout json` of a table report): `hikasa.table_compare.v1` with its `schema` key first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableReportJson {
    /// `hikasa.table_compare.v1`.
    pub schema: &'static str,
    /// The document.
    #[serde(flatten)]
    pub compare: TableCompareJson,
}

impl TableReportJson {
    /// The report of `src` under `view`.
    pub fn new(src: &TableSource<'_>, view: &TableViewOptions) -> TableReportJson {
        TableReportJson {
            schema: super::TABLE_COMPARE_SCHEMA,
            compare: table_compare_json(src, view),
        }
    }
}

fn side_json(src: &TableSource<'_>, side: Side) -> TableSideJson {
    let s: &TableSide = src.side(side);
    let wb = src.comparison.workbook(side);
    TableSideJson {
        spec: s.spec.clone(),
        size: s.size,
        mtime: s.mtime.map(|t| t.to_string()),
        source: wb.source().as_str(),
        sheets: wb.sheets().len() as u64,
        encoding: wb.encoding().map(str::to_owned),
        format: s.format.clone(),
    }
}

fn format_json(src: &TableSource<'_>, side: Side) -> TableFormatJson {
    let wb = src.comparison.workbook(side);
    let f = wb.detected();
    let spreadsheet = wb.source().is_spreadsheet();
    let (kind, delimiters, qualifier, consecutive, trim, widths) = match (&f.kind, spreadsheet) {
        (_, true) | (TableKind::Spreadsheet, _) => {
            ("spreadsheet", Vec::new(), None, false, false, Vec::new())
        }
        (TableKind::Delimited(d), false) => (
            "delimited",
            d.delimiters.iter().map(char::to_string).collect(),
            d.qualifier.map(String::from),
            d.consecutive_as_one,
            d.trim_whitespace,
            Vec::new(),
        ),
        (TableKind::Fixed(x), false) => ("fixed", Vec::new(), None, false, false, x.widths.clone()),
        (TableKind::Detect, false) => ("detect", Vec::new(), None, false, false, Vec::new()),
    };
    let r = &f.regional;
    TableFormatJson {
        kind,
        delimiters,
        qualifier,
        consecutive_as_one: consecutive,
        trim_whitespace: trim,
        widths,
        first_line: f.first_line.as_str(),
        regional: RegionalJson {
            decimal: r.decimal.to_string(),
            thousands: r.thousands.map(String::from),
            date_order: r.date_order.as_str(),
            date_sep: r.date_sep.map(String::from),
        },
        encoding: wb
            .encoding()
            .map(str::to_owned)
            .or_else(|| (!spreadsheet).then(|| f.encoding.clone()).flatten()),
        detected: src.side(side).sniffed,
    }
}

fn options_json(src: &TableSource<'_>) -> TableOptionsJson {
    let o = src.options;
    let pairs = match &o.sheets {
        SheetAlignment::Custom(pairs) => pairs
            .iter()
            .map(|p| SheetPairJson {
                left: p.left.as_ref().map(selector_text),
                right: p.right.as_ref().map(selector_text),
            })
            .collect(),
        _ => Vec::new(),
    };
    TableOptionsJson {
        sheets: SheetsOptionJson {
            mode: sheets_word(&o.sheets),
            pairs,
        },
        compare_formulas: o.compare_formulas,
    }
}

const fn sheet_state_word(s: hikasa_table::SheetState) -> &'static str {
    s.as_str()
}

fn alignment_json(sheet: &SheetComparison) -> AlignmentUsedJson {
    match sheet.alignment() {
        RowAlignmentUsed::Keyed { keys } => AlignmentUsedJson {
            mode: "keyed",
            keys: crate::table::key_names(sheet, keys),
            algorithm: None,
            sorted: false,
        },
        RowAlignmentUsed::Engine { algorithm, sorted } => AlignmentUsedJson {
            mode: "engine",
            keys: Vec::new(),
            algorithm: Some(algorithm.as_str()),
            sorted: *sorted,
        },
        RowAlignmentUsed::Orphan => AlignmentUsedJson {
            mode: "orphan",
            keys: Vec::new(),
            algorithm: None,
            sorted: false,
        },
    }
}

/// The header name of column `c` of `side`, when the comparison read a header there.
fn header_name(
    src: &TableSource<'_>,
    pair: usize,
    sheet: &SheetComparison,
    c: Option<u32>,
    side: Side,
) -> Option<String> {
    let c = c?;
    if !sheet.has_header(side) {
        return None;
    }
    let s = src.comparison.sheet(pair, side)?;
    s.header()
        .and_then(|h| h.get(c as usize).cloned())
        .or_else(|| Some(s.column_name(c).into_owned()))
}

fn column_json(
    src: &TableSource<'_>,
    pair: usize,
    sheet: &SheetComparison,
    k: usize,
    col: &ColumnPair,
) -> TableColumnJson {
    let side = |c: Option<u32>, s: Side| -> Option<ColumnSideJson> {
        let c = c?;
        let letter = src
            .comparison
            .sheet(pair, s)
            .map_or_else(|| hikasa_table::column_letters(c), |sh| sh.column_label(c));
        Some(ColumnSideJson {
            index: c + 1,
            letter,
            name: header_name(src, pair, sheet, Some(c), s),
        })
    };
    TableColumnJson {
        index: k as u32 + 1,
        name: col.name.clone(),
        left: side(col.left, Side::Left),
        right: side(col.right, Side::Right),
        key: col.key.map(|k| u32::from(k) + 1),
        rule: ColumnRuleJson::new(&col.rule),
        kind_left: col.left.map(|_| col.left_kind.as_str()),
        kind_right: col.right.map(|_| col.right_kind.as_str()),
        hidden: col.hidden,
    }
}

fn cell_json(src: &TableSource<'_>, pair: usize, row: u32, column: u32, side: Side) -> CellJson {
    let value: Value<'_> = src.comparison.cell(pair, row, column, side);
    CellJson {
        value: value.display().into_owned(),
        kind: value.kind().as_str(),
        formula: src
            .comparison
            .formula(pair, row, column, side)
            .map(str::to_owned),
    }
}

/// The cell pair of `column` in row `row` (a side without the row or the column is `null`).
fn cell_pair(
    src: &TableSource<'_>,
    pair: usize,
    sheet: &SheetComparison,
    row: u32,
    column: u32,
    state: CellState,
) -> CellPairJson {
    let r = &sheet.rows()[row as usize];
    let col = &sheet.columns()[column as usize];
    let side = |s: Side, c: Option<u32>| {
        r.side(s)?;
        c?;
        Some(cell_json(src, pair, row, column, s))
    };
    CellPairJson {
        column: column + 1,
        state: state.as_str(),
        left: side(Side::Left, col.left),
        right: side(Side::Right, col.right),
    }
}

fn row_json(
    src: &TableSource<'_>,
    pair: usize,
    sheet: &SheetComparison,
    row: u32,
    cells: TableCells,
) -> TableRowJson {
    let r = &sheet.rows()[row as usize];
    let mut out = Vec::new();
    match (r.kind(), cells) {
        (_, TableCells::All) => {
            for k in 0..sheet.columns().len() as u32 {
                let state = match r.kind() {
                    RowKind::Same => CellState::Same,
                    _ => sheet.cell_state(row, k),
                };
                out.push(cell_pair(src, pair, sheet, row, k, state));
            }
        }
        (RowKind::Changed, TableCells::Differing) => {
            for d in sheet.differing_cells(row) {
                out.push(cell_pair(src, pair, sheet, row, d.column, d.state));
            }
        }
        (RowKind::LeftOnly | RowKind::RightOnly, TableCells::Differing) => {
            let side = if r.kind() == RowKind::LeftOnly {
                Side::Left
            } else {
                Side::Right
            };
            for k in 0..sheet.columns().len() as u32 {
                if matches!(src.comparison.cell(pair, row, k, side), Value::Empty) {
                    continue;
                }
                out.push(cell_pair(
                    src,
                    pair,
                    sheet,
                    row,
                    k,
                    sheet.cell_state(row, k),
                ));
            }
        }
        (RowKind::Same, TableCells::Differing) => {}
    }
    TableRowJson {
        row: row + 1,
        state: r.kind().as_str(),
        importance: match r.importance() {
            Importance::None => "none",
            Importance::Unimportant => "unimportant",
            Importance::Important => "important",
        },
        left: src.comparison.row_label(pair, row, Side::Left),
        right: src.comparison.row_label(pair, row, Side::Right),
        cells: out,
    }
}

fn sheet_json(
    src: &TableSource<'_>,
    pair: usize,
    sheet: &SheetComparison,
    view: &TableViewOptions,
) -> TableSheetJson {
    let filter = RowFilter {
        ignore_unimportant: view.ignore_unimportant,
        ..view.filter
    };
    let sheet_ref = |at: Option<u32>, side: Side| -> Option<SheetRefJson> {
        let s = src.comparison.sheet(pair, side)?;
        Some(SheetRefJson {
            index: at? + 1,
            name: s.name().to_owned(),
        })
    };
    let header = (!sheet.header_states().is_empty()).then(|| {
        sheet
            .header_states()
            .iter()
            .zip(sheet.columns())
            .enumerate()
            .map(|(k, (state, col))| HeaderCellJson {
                column: k as u32 + 1,
                state: state.as_str(),
                left: header_name(src, pair, sheet, col.left, Side::Left),
                right: header_name(src, pair, sheet, col.right, Side::Right),
            })
            .collect()
    });
    let mut rows = Vec::new();
    let mut hidden = 0u64;
    for (i, r) in sheet.rows().iter().enumerate() {
        if filter.shows(r) {
            rows.push(row_json(src, pair, sheet, i as u32, view.cells));
        } else {
            hidden += 1;
        }
    }
    let filter_word = match display_word(filter) {
        "mismatches" => "differences",
        "matches" => "same",
        other => other,
    };
    TableSheetJson {
        name: sheet.name().to_owned(),
        left: sheet_ref(sheet.left(), Side::Left),
        right: sheet_ref(sheet.right(), Side::Right),
        renamed: sheet.renamed(),
        state: sheet_state_word(sheet.state()),
        alignment: alignment_json(sheet),
        columns: sheet
            .columns()
            .iter()
            .enumerate()
            .map(|(k, c)| column_json(src, pair, sheet, k, c))
            .collect(),
        stats: SheetStatsJson::new(sheet.stats()),
        duplicates: DuplicatesJson::new(sheet.duplicates()),
        header,
        display: TableDisplayJson {
            filter: filter_word,
            toggles: TogglesJson {
                same: filter.same,
                changed: filter.changed,
                left_only: filter.left_only,
                right_only: filter.right_only,
            },
            cells: view.cells.as_str(),
        },
        rows,
        hidden_rows: hidden,
    }
}

/// The JSON of a table warning.
pub fn table_warning_json(w: &TableWarning) -> TableWarningJson {
    TableWarningJson {
        code: w.code.as_str(),
        message: w.message.clone(),
        side: w.side.map(Side::as_str),
        sheet: w.sheet.clone(),
    }
}

/// Builds `hikasa.table_compare.v1` (§6.3).
pub fn table_compare_json(src: &TableSource<'_>, view: &TableViewOptions) -> TableCompareJson {
    let cmp = src.comparison;
    let stats = cmp.stats();
    TableCompareJson {
        left: side_json(src, Side::Left),
        right: side_json(src, Side::Right),
        format: TableFormatsJson {
            left: format_json(src, Side::Left),
            right: format_json(src, Side::Right),
        },
        options: options_json(src),
        ignore_unimportant: view.ignore_unimportant,
        verdict: src.verdict(view.ignore_unimportant),
        stats: TableStatsJson {
            sheets_paired: stats.sheets_paired,
            sheets_left_only: stats.sheets_left_only,
            sheets_right_only: stats.sheets_right_only,
            sheets_differing: stats.sheets_differing,
            totals: SheetStatsJson::new(&stats.totals),
        },
        sheets: cmp
            .sheets()
            .iter()
            .enumerate()
            .map(|(i, s)| sheet_json(src, i, s, view))
            .collect(),
        warnings: cmp.warnings().iter().map(table_warning_json).collect(),
    }
}
