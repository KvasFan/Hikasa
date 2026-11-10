//! The table model (contract §4): [`Workbook`], [`Sheet`], [`SheetInfo`], [`ColumnKind`] and the
//! [`SheetBuilder`] the loaders fill.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::cell::{Cell, CellKind, FLAG_FORMULA, FLAG_TAIL, Value};
use crate::error::{TableError, TableWarning};
use crate::format::TableFormat;
use crate::regional::Regional;

/// Where a workbook came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Delimited text.
    Delimited,
    /// Fixed-width text.
    Fixed,
    /// Office Open XML workbook.
    Xlsx,
    /// Office Open XML workbook with macros.
    Xlsm,
    /// Binary Office Open XML workbook.
    Xlsb,
    /// BIFF workbook (Excel 97–2003).
    Xls,
    /// OpenDocument spreadsheet.
    Ods,
}

impl SourceKind {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SourceKind::Delimited => "delimited",
            SourceKind::Fixed => "fixed",
            SourceKind::Xlsx => "xlsx",
            SourceKind::Xlsm => "xlsm",
            SourceKind::Xlsb => "xlsb",
            SourceKind::Xls => "xls",
            SourceKind::Ods => "ods",
        }
    }

    /// True for the workbook formats.
    pub const fn is_spreadsheet(self) -> bool {
        !matches!(self, SourceKind::Delimited | SourceKind::Fixed)
    }
}

/// Detected kind of a column (over all its non-empty data cells; header excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    /// No non-empty cell.
    Empty,
    /// Every non-empty cell is (or parses as) a number.
    Number,
    /// … a date-time.
    DateTime,
    /// … a duration (workbooks only).
    Duration,
    /// … a boolean.
    Bool,
    /// Text, or a mix that includes text.
    Text,
    /// Typed cells of several kinds and no text (workbooks).
    Mixed,
}

impl ColumnKind {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ColumnKind::Empty => "empty",
            ColumnKind::Number => "number",
            ColumnKind::DateTime => "date_time",
            ColumnKind::Duration => "duration",
            ColumnKind::Bool => "bool",
            ColumnKind::Text => "text",
            ColumnKind::Mixed => "mixed",
        }
    }

    /// The kind of a column holding cells of kinds `self` and `other`.
    pub(crate) fn join(self, other: ColumnKind) -> ColumnKind {
        use ColumnKind as K;
        match (self, other) {
            (K::Empty, x) | (x, K::Empty) => x,
            (a, b) if a == b => a,
            (K::Text, _) | (_, K::Text) => K::Text,
            _ => K::Mixed,
        }
    }
}

/// A merged region, 0-based inclusive, in sheet coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Region {
    /// First row.
    pub first_row: u32,
    /// First column.
    pub first_col: u32,
    /// Last row (inclusive).
    pub last_row: u32,
    /// Last column (inclusive).
    pub last_col: u32,
}

/// Facts about a sheet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetInfo {
    /// The sheet is hidden in its workbook.
    pub hidden: bool,
    /// Sheet coordinates (0-based row, column) of the first stored row's first cell (the used range's top-left;
    /// the header row when there is one). `(0, 0)` for text.
    pub origin: (u32, u32),
    /// Merged regions (workbooks).
    pub merged: Vec<Region>,
    /// 1904 date system (workbooks).
    pub date_1904: bool,
    /// Hidden rows: sorted, merged, inclusive ranges of sheet rows (0-based). Read from xlsx and xlsm; xlsb, xls,
    /// ods and text report none (F13).
    pub hidden_rows: Vec<(u32, u32)>,
    /// Hidden columns: inclusive ranges of sheet columns (0-based), as `hidden_rows`.
    pub hidden_columns: Vec<(u32, u32)>,
}

/// True when `x` lies in one of the sorted inclusive `ranges`.
fn in_ranges(ranges: &[(u32, u32)], x: u32) -> bool {
    let i = ranges.partition_point(|r| r.1 < x);
    ranges.get(i).is_some_and(|r| r.0 <= x)
}

/// One sheet: ragged rows of packed cells over a text arena (T-5).
#[derive(Debug, Clone)]
pub struct Sheet {
    name: String,
    info: SheetInfo,
    arena: String,
    cells: Vec<Cell>,
    /// Sparse sheets (F4): the column of each cell (sorted within a row); empty for dense sheets, whose cell `i` of
    /// a row is column `i`.
    cols: Vec<u32>,
    /// `rows[r]..rows[r + 1]` are the cells of stored row `r` (header included as stored row 0 when present).
    rows: Vec<u32>,
    /// 1-based first line of each stored row (text), empty for workbooks.
    lines: Vec<u32>,
    /// Sheet row (0-based) of each stored row for workbooks when rows are not consecutive; empty = consecutive.
    sheet_rows: Vec<u32>,
    header: Option<Vec<String>>,
    kinds: Vec<ColumnKind>,
    /// Formulas: (stored row, column) → text, sorted.
    formulas: Vec<((u32, u32), Box<str>)>,
    regional: Regional,
}

/// A loaded table: one sheet for text, every sheet for workbooks.
#[derive(Debug, Clone)]
pub struct Workbook {
    pub(crate) sheets: Vec<Sheet>,
    pub(crate) source: SourceKind,
    pub(crate) detected: TableFormat,
    pub(crate) warnings: Vec<TableWarning>,
    pub(crate) encoding: Option<&'static str>,
}

impl Workbook {
    /// Sheets in workbook order.
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// Where the table came from.
    pub fn source(&self) -> SourceKind {
        self.source
    }

    /// The effective format after detection (sniffed delimiters, header choice).
    pub fn detected(&self) -> &TableFormat {
        &self.detected
    }

    /// Load warnings.
    pub fn warnings(&self) -> &[TableWarning] {
        &self.warnings
    }

    /// Encoding name of a text table (`None` for workbooks).
    pub fn encoding(&self) -> Option<&'static str> {
        self.encoding
    }

    /// Builds a workbook from sheets (tests and callers that assemble tables in memory).
    pub fn from_sheets(sheets: Vec<Sheet>, source: SourceKind) -> Workbook {
        Workbook {
            sheets,
            source,
            detected: TableFormat::default(),
            warnings: Vec::new(),
            encoding: None,
        }
    }
}

/// The *Spreadsheet text* conversion (contract T-22): every sheet as tab-separated display texts (fields with a tab,
/// a line break or a `"` quoted, `"` doubled), a `--- <sheet name> ---` line before each sheet when there are several.
pub fn spreadsheet_to_text(workbook: &Workbook) -> String {
    let opts = crate::format::DelimitedOptions::with('\t');
    let several = workbook.sheets.len() > 1;
    let mut out = String::new();
    for sheet in &workbook.sheets {
        if several {
            out.push_str("--- ");
            out.push_str(sheet.name());
            out.push_str(" ---\n");
        }
        let rows: Vec<Vec<String>> = (0..sheet.stored_count())
            .map(|r| {
                sheet
                    .stored_row(r)
                    .values()
                    .map(|v| v.display().into_owned())
                    .collect()
            })
            .collect();
        // A qualifier is set, so nothing is lost.
        out.push_str(&crate::delimited::write_delimited(&rows, &opts).unwrap_or_default());
    }
    out
}

/// A row of a sheet: dense (cell `i` is column `i`) or sparse (each stored cell has its column; F4).
#[derive(Debug, Clone, Copy)]
pub struct RowView<'a> {
    cells: &'a [Cell],
    cols: &'a [u32],
    arena: &'a str,
}

impl<'a> RowView<'a> {
    /// Columns up to the row's last stored cell (ragged), gaps included.
    pub fn len(&self) -> u32 {
        match self.cols.last() {
            Some(&last) => last + 1,
            None if self.cols.is_empty() && !self.cells.is_empty() => self.cells.len() as u32,
            None => 0,
        }
    }

    /// True for a row without cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// True when the row stores its cells with their columns (a gap costs nothing).
    pub fn is_sparse(&self) -> bool {
        !self.cols.is_empty()
    }

    /// Cells actually stored (gaps of a sparse row excluded).
    pub fn stored_len(&self) -> u32 {
        self.cells.len() as u32
    }

    fn index(&self, c: u32) -> Option<usize> {
        if self.cols.is_empty() {
            ((c as usize) < self.cells.len()).then_some(c as usize)
        } else {
            self.cols.binary_search(&c).ok()
        }
    }

    /// Value of column `c` (`Empty` past the end and in gaps).
    pub fn get(&self, c: u32) -> Value<'a> {
        self.index(c)
            .map_or(Value::Empty, |i| self.cells[i].value(self.arena))
    }

    /// The packed cell of column `c` (`Cell::EMPTY` past the end and in gaps).
    pub fn cell(&self, c: u32) -> Cell {
        self.index(c).map_or(Cell::EMPTY, |i| self.cells[i])
    }

    /// Every value from column 0 to the last stored one, gaps as `Empty`.
    pub fn values(&self) -> impl Iterator<Item = Value<'a>> + 'a {
        let row = *self;
        let mut next = 0usize;
        (0..self.len()).map(move |c| {
            if row.cols.is_empty() {
                return row.cells[c as usize].value(row.arena);
            }
            if row.cols.get(next) == Some(&c) {
                next += 1;
                row.cells[next - 1].value(row.arena)
            } else {
                Value::Empty
            }
        })
    }

    /// The stored cells with their columns, in column order (a dense row stores every column up to its end).
    pub fn stored(&self) -> impl Iterator<Item = (u32, Value<'a>)> + 'a {
        let row = *self;
        self.cells.iter().enumerate().map(move |(i, cell)| {
            let c = if row.cols.is_empty() {
                i as u32
            } else {
                row.cols[i]
            };
            (c, cell.value(row.arena))
        })
    }

    /// The packed stored cells with their columns.
    pub(crate) fn stored_cells(&self) -> impl Iterator<Item = (u32, Cell)> + 'a {
        let row = *self;
        self.cells.iter().enumerate().map(move |(i, cell)| {
            let c = if row.cols.is_empty() {
                i as u32
            } else {
                row.cols[i]
            };
            (c, *cell)
        })
    }
}

/// Column letters of a 0-based column (`0` → `A`, `26` → `AA`).
pub fn column_letters(mut c: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// 0-based column of letters (`A` → 0); `None` for anything but 1–3 ASCII letters up to `XFD`.
pub fn parse_column_letters(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for b in s.bytes() {
        n = n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1;
    }
    (n <= 16_384).then(|| n - 1)
}

impl Sheet {
    /// Sheet name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Facts about the sheet.
    pub fn info(&self) -> &SheetInfo {
        &self.info
    }

    fn header_rows(&self) -> u32 {
        u32::from(self.header.is_some())
    }

    /// Data rows (the header excluded).
    pub fn row_count(&self) -> u32 {
        (self.rows.len() as u32).saturating_sub(1) - self.header_rows()
    }

    /// The widest stored row (header included).
    pub fn column_count(&self) -> u32 {
        self.kinds.len() as u32
    }

    fn stored(&self, stored: u32) -> RowView<'_> {
        let s = stored as usize;
        let (a, b) = match (self.rows.get(s), self.rows.get(s + 1)) {
            (Some(&a), Some(&b)) => (a as usize, b as usize),
            _ => (0, 0),
        };
        RowView {
            cells: &self.cells[a..b],
            cols: if self.cols.is_empty() {
                &[]
            } else {
                &self.cols[a..b]
            },
            arena: &self.arena,
        }
    }

    /// Data row `r` (0-based, header excluded).
    pub fn row(&self, r: u32) -> RowView<'_> {
        self.stored(r + self.header_rows())
    }

    /// Value of data row `r`, column `c`.
    pub fn cell(&self, r: u32, c: u32) -> Value<'_> {
        self.row(r).get(c)
    }

    /// Header names, when the first line is a header.
    pub fn header(&self) -> Option<&[String]> {
        self.header.as_deref()
    }

    /// Header name of column `c`, or its letters.
    pub fn column_name(&self, c: u32) -> Cow<'_, str> {
        match self.header.as_ref().and_then(|h| h.get(c as usize)) {
            Some(name) if !name.trim().is_empty() => Cow::Borrowed(name),
            _ => Cow::Owned(self.column_label(c)),
        }
    }

    /// Letters of column `c` in the sheet's own coordinates.
    pub fn column_label(&self, c: u32) -> String {
        column_letters(self.info.origin.1 + c)
    }

    /// Detected kind of column `c`.
    pub fn column_kind(&self, c: u32) -> ColumnKind {
        self.kinds
            .get(c as usize)
            .copied()
            .unwrap_or(ColumnKind::Empty)
    }

    /// Formula of data row `r`, column `c`.
    pub fn formula(&self, r: u32, c: u32) -> Option<&str> {
        self.stored_formula(r + self.header_rows(), c)
    }

    /// 1-based label of data row `r`: the first line (text) or the sheet's row number (workbooks).
    pub fn row_label(&self, r: u32) -> u64 {
        self.stored_label(r + self.header_rows())
    }

    /// True when data row `r` is hidden in its workbook (F13).
    pub fn row_hidden(&self, r: u32) -> bool {
        self.stored_hidden(r + self.header_rows())
    }

    /// True when stored row `stored` is hidden.
    pub(crate) fn stored_hidden(&self, stored: u32) -> bool {
        !self.info.hidden_rows.is_empty()
            && in_ranges(
                &self.info.hidden_rows,
                (self.stored_label(stored).saturating_sub(1)) as u32,
            )
    }

    /// True when column `c` is hidden in its workbook (F13).
    pub fn column_hidden(&self, c: u32) -> bool {
        !self.info.hidden_columns.is_empty()
            && in_ranges(&self.info.hidden_columns, self.info.origin.1 + c)
    }

    /// Facts about the sheet, mutably (the loaders add what they read after the cells).
    pub(crate) fn info_mut(&mut self) -> &mut SheetInfo {
        &mut self.info
    }

    /// The regional settings the sheet's text cells are interpreted with.
    pub fn regional(&self) -> &Regional {
        &self.regional
    }

    /// Approximate heap bytes of the sheet (the memory measurements of T-15).
    pub fn heap_bytes(&self) -> usize {
        self.arena.capacity()
            + self.cells.capacity() * std::mem::size_of::<Cell>()
            + (self.cols.capacity()
                + self.rows.capacity()
                + self.lines.capacity()
                + self.sheet_rows.capacity())
                * 4
            + self
                .formulas
                .iter()
                .map(|(_, f)| 16 + f.len())
                .sum::<usize>()
    }

    /// The merged region covering data row `r`, column `c` (workbooks), in sheet coordinates.
    pub fn merged_region(&self, r: u32, c: u32) -> Option<Region> {
        let row = self.info.origin.0 + r + self.header_rows();
        let col = self.info.origin.1 + c;
        self.info
            .merged
            .iter()
            .find(|g| {
                (g.first_row..=g.last_row).contains(&row)
                    && (g.first_col..=g.last_col).contains(&col)
            })
            .copied()
    }

    /// Adds formulas given in sheet coordinates (the second pass of xlsb, xls and ods). `first_row` is the
    /// sheet row of stored row 0.
    pub(crate) fn add_formulas(&mut self, formulas: Vec<((u32, u32), Box<str>)>, first_row: u32) {
        for ((r, c), f) in formulas {
            let (Some(stored), Some(col)) =
                (r.checked_sub(first_row), c.checked_sub(self.info.origin.1))
            else {
                continue;
            };
            if stored >= self.stored_count() {
                continue;
            }
            let start = self.rows[stored as usize] as usize;
            let end = self.rows[stored as usize + 1] as usize;
            let at = if self.cols.is_empty() {
                (start + (col as usize) < end).then_some(start + col as usize)
            } else {
                self.cols[start..end]
                    .binary_search(&col)
                    .ok()
                    .map(|i| start + i)
            };
            if let Some(at) = at {
                let cell = &mut self.cells[at];
                *cell = cell.with_flag(FLAG_FORMULA);
            }
            self.formulas.push(((stored, col), f));
        }
        self.formulas.sort_by_key(|f| f.0);
    }

    /// The header row's stored cells (when there is a header).
    pub fn header_row(&self) -> Option<RowView<'_>> {
        self.header.as_ref().map(|_| self.stored(0))
    }
}

/// Builds a [`Sheet`] row by row.
#[derive(Debug)]
pub struct SheetBuilder {
    name: String,
    info: SheetInfo,
    arena: String,
    tail: String,
    cells: Vec<Cell>,
    /// The column of each cell when the builder is sparse (workbooks).
    cols: Vec<u32>,
    sparse: bool,
    rows: Vec<u32>,
    lines: Vec<u32>,
    sheet_rows: Vec<u32>,
    formulas: Vec<((u32, u32), Box<str>)>,
    regional: Regional,
    trim_leading: bool,
    /// Shared strings already in the tail: the string's address → its span (F3: one copy per sheet).
    shared: std::collections::HashMap<usize, (u32, u32)>,
    /// The last text appended (offset, length): an equal text that follows reuses it.
    last_text: Option<(u32, u32)>,
    /// Most bytes the tail may hold (the workbook's text budget left for this sheet).
    text_limit: u64,
}

impl SheetBuilder {
    /// A builder whose text cells are spans of `arena` (the decoded text of a delimited file; empty for
    /// workbooks, whose texts are appended with [`SheetBuilder::push_text`]).
    pub fn new(name: impl Into<String>, arena: String, regional: Regional) -> SheetBuilder {
        SheetBuilder {
            name: name.into(),
            info: SheetInfo::default(),
            arena,
            tail: String::new(),
            cells: Vec::new(),
            cols: Vec::new(),
            sparse: false,
            rows: vec![0],
            lines: Vec::new(),
            sheet_rows: Vec::new(),
            formulas: Vec::new(),
            regional,
            trim_leading: false,
            shared: std::collections::HashMap::new(),
            last_text: None,
            text_limit: u64::from(u32::MAX),
        }
    }

    /// Makes the builder sparse: cells are pushed at their columns ([`SheetBuilder::push_at`]) and gaps cost
    /// nothing (F4). [`SheetBuilder::finish`] keeps the sparse form only when it is the smaller one.
    pub(crate) fn set_sparse(&mut self) {
        self.sparse = true;
    }

    /// Limits the bytes of text the builder may hold (F3).
    pub(crate) fn set_text_limit(&mut self, limit: u64) {
        self.text_limit = limit.min(u64::from(u32::MAX));
    }

    /// Bytes of text appended so far (shared strings counted once).
    pub(crate) fn text_bytes(&self) -> u64 {
        self.tail.len() as u64
    }

    /// Sheet facts.
    pub fn info_mut(&mut self) -> &mut SheetInfo {
        &mut self.info
    }

    /// The arena (for the delimited parser, which reads it while it pushes spans).
    pub fn arena(&self) -> &str {
        &self.arena
    }

    /// The arena, mutably (the parsers take it out while they push spans of it, then put it back).
    pub(crate) fn arena_mut(&mut self) -> &mut String {
        &mut self.arena
    }

    /// Reserves room for `cells` more cells.
    pub fn reserve(&mut self, cells: usize, rows: usize) {
        self.cells.reserve(cells);
        self.rows.reserve(rows);
    }

    fn check_cells(&self) -> Result<(), TableError> {
        if self.cells.len() >= u32::MAX as usize {
            return Err(TableError::TooLarge {
                name: self.name.clone(),
                what: "cells",
                limit: u64::from(u32::MAX),
            });
        }
        Ok(())
    }

    /// Pushes a cell to the current row (the next column).
    pub fn push(&mut self, cell: Cell) -> Result<(), TableError> {
        self.check_cells()?;
        if self.sparse {
            let col = self.current_len();
            self.cols.push(col);
        }
        self.cells.push(cell);
        Ok(())
    }

    /// Pushes a cell at column `col` of the current row (at or past [`SheetBuilder::current_len`]); a dense builder
    /// pads the gap with empty cells.
    pub(crate) fn push_at(&mut self, col: u32, cell: Cell) -> Result<(), TableError> {
        self.check_cells()?;
        if self.sparse {
            self.cols.push(col);
            self.cells.push(cell);
            return Ok(());
        }
        for _ in self.current_len()..col {
            self.push(Cell::EMPTY)?;
        }
        self.push(cell)
    }

    /// Pushes a text cell that is the span `[start, end)` of the arena.
    pub fn push_span(&mut self, start: usize, end: usize) -> Result<(), TableError> {
        self.push(Cell::text(start as u32, (end - start) as u32))
    }

    /// Pushes a text cell whose text is not in the arena (unescaped text, workbook strings).
    pub fn push_text(&mut self, text: &str) -> Result<(), TableError> {
        let cell = self.text_cell(text)?;
        self.push(cell)
    }

    /// Appends `text` to the tail (an equal text just appended is reused) and returns its cell, not pushed.
    pub(crate) fn text_cell(&mut self, text: &str) -> Result<Cell, TableError> {
        if text.is_empty() {
            return Ok(Cell::text(0, 0));
        }
        if let Some((offset, len)) = self.last_text
            && self.tail.get(offset as usize..(offset + len) as usize) == Some(text)
        {
            return Ok(Cell::text(offset, len).with_flag(FLAG_TAIL));
        }
        let offset = self.tail.len();
        let total = offset as u64 + text.len() as u64 + self.arena.len() as u64;
        if total > u64::from(u32::MAX) || offset as u64 + text.len() as u64 > self.text_limit {
            return Err(TableError::TooLarge {
                name: self.name.clone(),
                what: "bytes of text",
                limit: self.text_limit.min(u64::from(u32::MAX)),
            });
        }
        self.tail.push_str(text);
        let span = (offset as u32, text.len() as u32);
        self.last_text = Some(span);
        Ok(Cell::text(span.0, span.1).with_flag(FLAG_TAIL))
    }

    /// The cell of a shared string (one entry of a workbook's string table, identified by its address): its text
    /// is copied into the sheet once, every cell that uses it points at that copy (F3).
    pub(crate) fn shared_text_cell(&mut self, text: &str) -> Result<Cell, TableError> {
        if text.is_empty() {
            return Ok(Cell::text(0, 0));
        }
        let key = text.as_ptr() as usize;
        if let Some(&(offset, len)) = self.shared.get(&key)
            && len as usize == text.len()
        {
            return Ok(Cell::text(offset, len).with_flag(FLAG_TAIL));
        }
        let cell = self.text_cell(text)?;
        self.shared.insert(key, cell.span());
        Ok(cell)
    }

    /// Marks the last pushed cell as having `formula`.
    pub fn set_formula(&mut self, formula: &str) {
        let stored = (self.rows.len() - 1) as u32;
        let col = self.current_len().saturating_sub(1);
        if let Some(cell) = self.cells.last_mut() {
            *cell = cell.with_flag(FLAG_FORMULA);
        }
        self.formulas.push(((stored, col), formula.into()));
    }

    /// Columns of the current (open) row so far.
    pub fn current_len(&self) -> u32 {
        let start = *self.rows.last().unwrap_or(&0) as usize;
        if self.sparse {
            if self.cells.len() > start {
                self.cols.last().map_or(0, |c| c + 1)
            } else {
                0
            }
        } else {
            (self.cells.len() - start) as u32
        }
    }

    /// Ends the current row (`line`: 1-based first line for text).
    pub fn end_row(&mut self, line: Option<u32>) {
        // Ragged rows: trailing empty cells are not stored.
        let start = *self.rows.last().unwrap_or(&0) as usize;
        while self.cells.len() > start
            && self
                .cells
                .last()
                .is_some_and(|c| c.kind() == CellKind::Empty && !c.has_formula())
        {
            self.cells.pop();
            if self.sparse {
                self.cols.pop();
            }
        }
        self.rows.push(self.cells.len() as u32);
        if let Some(line) = line {
            self.lines.push(line);
        }
    }

    /// Records the 0-based sheet row of the row just ended (workbooks with gaps).
    pub fn set_sheet_row(&mut self, row: u32) {
        self.sheet_rows.push(row);
    }

    /// Stored rows so far.
    pub fn row_count(&self) -> u32 {
        (self.rows.len() - 1) as u32
    }

    /// Finishes the sheet: resolves tail spans, detects the header (§2.5) and the column kinds.
    pub fn finish(mut self, first_line: crate::format::FirstLine) -> Sheet {
        let base = self.arena.len() as u32;
        if !self.tail.is_empty() {
            self.arena.push_str(&self.tail);
        }
        self.tail = String::new();
        for cell in &mut self.cells {
            if cell.has_flag(FLAG_TAIL) {
                let (offset, _) = cell.span();
                cell.set_offset(offset + base);
                cell.clear_flag(FLAG_TAIL);
            }
        }
        self.formulas.sort_by_key(|f| f.0);
        self.shared = std::collections::HashMap::new();
        if self.trim_leading {
            self.trim_leading_columns();
        }
        if self.sparse {
            self.densify_when_smaller();
        }
        // Consecutive sheet rows need no table.
        if self
            .sheet_rows
            .iter()
            .enumerate()
            .all(|(i, &r)| r == self.info.origin.0 + i as u32)
        {
            self.sheet_rows = Vec::new();
        }
        let mut sheet = Sheet {
            name: self.name,
            info: self.info,
            arena: self.arena,
            cells: self.cells,
            cols: self.cols,
            rows: self.rows,
            lines: self.lines,
            sheet_rows: self.sheet_rows,
            header: None,
            kinds: Vec::new(),
            formulas: self.formulas,
            regional: self.regional,
        };
        sheet.arena.shrink_to_fit();
        sheet.cells.shrink_to_fit();
        sheet.cols.shrink_to_fit();
        let width = (0..sheet.rows.len().saturating_sub(1) as u32)
            .map(|r| sheet.stored(r).len())
            .max()
            .unwrap_or(0);
        let has_header = crate::header::detect_header(&sheet, first_line, width);
        if has_header {
            let names = (0..width)
                .map(|c| sheet.stored(0).get(c).display().trim().to_owned())
                .collect();
            sheet.header = Some(names);
        }
        sheet.kinds = crate::header::column_kinds(&sheet, width);
        sheet
    }

    /// Workbooks: columns that are empty in every row are dropped from the left (the origin moves).
    pub(crate) fn set_trim_leading_columns(&mut self) {
        self.trim_leading = true;
    }

    fn trim_leading_columns(&mut self) {
        let rows = self.rows.len() - 1;
        let mut k = u32::MAX;
        for r in 0..rows {
            let (a, b) = (self.rows[r] as usize, self.rows[r + 1] as usize);
            if a == b {
                continue;
            }
            let Some(lead) = self.cells[a..b]
                .iter()
                .position(|c| c.kind() != CellKind::Empty || c.has_formula())
            else {
                continue;
            };
            let lead = if self.sparse {
                self.cols.get(a + lead).copied().unwrap_or(u32::MAX)
            } else {
                lead as u32
            };
            k = k.min(lead);
        }
        if k == 0 || k == u32::MAX {
            return;
        }
        if self.sparse {
            // Every stored cell of a column before `k` is an empty cell without formula: drop them.
            let mut out = Vec::with_capacity(self.cells.len());
            let mut cols = Vec::with_capacity(self.cols.len());
            let mut starts = Vec::with_capacity(self.rows.len());
            starts.push(0u32);
            for r in 0..rows {
                let (a, b) = (self.rows[r] as usize, self.rows[r + 1] as usize);
                for i in a..b {
                    if self.cols[i] >= k {
                        out.push(self.cells[i]);
                        cols.push(self.cols[i] - k);
                    }
                }
                starts.push(out.len() as u32);
            }
            self.cells = out;
            self.cols = cols;
            self.rows = starts;
            for ((_, c), _) in &mut self.formulas {
                *c -= k;
            }
            self.info.origin.1 += k;
            return;
        }
        let mut out = Vec::with_capacity(self.cells.len());
        let mut starts = Vec::with_capacity(self.rows.len());
        starts.push(0u32);
        for r in 0..rows {
            let (a, b) = (self.rows[r] as usize, self.rows[r + 1] as usize);
            if b > a {
                out.extend_from_slice(&self.cells[a + k as usize..b]);
            }
            starts.push(out.len() as u32);
        }
        self.cells = out;
        self.rows = starts;
        for ((_, c), _) in &mut self.formulas {
            *c -= k;
        }
        self.info.origin.1 += k;
    }
}

impl SheetBuilder {
    /// A sparse sheet becomes dense when the padding costs no more than the columns it saves (12 bytes per empty
    /// cell against 4 per stored cell): a few blanks inside rows stay O(1) to read, a far column stays cheap.
    fn densify_when_smaller(&mut self) {
        let rows = self.rows.len() - 1;
        let stored = self.cells.len() as u64;
        let mut logical = 0u64;
        for r in 0..rows {
            let (a, b) = (self.rows[r] as usize, self.rows[r + 1] as usize);
            if b > a {
                logical += u64::from(self.cols[b - 1]) + 1;
            }
        }
        let padding = logical - stored;
        if padding * 3 > stored {
            return;
        }
        let mut out = Vec::with_capacity(logical as usize);
        let mut starts = Vec::with_capacity(self.rows.len());
        starts.push(0u32);
        for r in 0..rows {
            let (a, b) = (self.rows[r] as usize, self.rows[r + 1] as usize);
            let base = out.len();
            for i in a..b {
                out.resize(base + self.cols[i] as usize, Cell::EMPTY);
                out.push(self.cells[i]);
            }
            starts.push(out.len() as u32);
        }
        self.cells = out;
        self.cols = Vec::new();
        self.rows = starts;
        self.sparse = false;
    }
}

impl Sheet {
    /// True when the sheet stores its rows sparsely (F4).
    pub fn is_sparse(&self) -> bool {
        !self.cols.is_empty()
    }

    /// The packed cells of stored row `stored` (header = stored row 0 when present).
    pub(crate) fn stored_row(&self, stored: u32) -> RowView<'_> {
        self.stored(stored)
    }

    /// Stored rows (header included).
    pub(crate) fn stored_count(&self) -> u32 {
        (self.rows.len() - 1) as u32
    }

    /// Formula of stored row `stored`, column `c`.
    pub(crate) fn stored_formula(&self, stored: u32, c: u32) -> Option<&str> {
        let key = (stored, c);
        self.formulas
            .binary_search_by(|(k, _)| k.cmp(&key))
            .ok()
            .map(|i| &*self.formulas[i].1)
    }

    /// 1-based label of stored row `stored` (line or sheet row).
    pub(crate) fn stored_label(&self, stored: u32) -> u64 {
        let s = stored as usize;
        if let Some(&line) = self.lines.get(s) {
            return u64::from(line);
        }
        if let Some(&row) = self.sheet_rows.get(s) {
            return u64::from(row) + 1;
        }
        u64::from(self.info.origin.0) + u64::from(stored) + 1
    }

    /// True when the sheet stores formulas.
    pub(crate) fn has_formulas(&self) -> bool {
        !self.formulas.is_empty()
    }

    /// Builds a sheet from rows of display texts (tests, conversions): every cell is text.
    pub fn from_text_rows<S: AsRef<str>>(
        name: &str,
        rows: &[Vec<S>],
        first_line: crate::format::FirstLine,
        regional: Regional,
    ) -> Sheet {
        let mut b = SheetBuilder::new(name, String::new(), regional);
        for (i, row) in rows.iter().enumerate() {
            for v in row {
                let v = v.as_ref();
                let _ = if v.is_empty() {
                    b.push(Cell::EMPTY)
                } else {
                    b.push_text(v)
                };
            }
            b.end_row(Some(i as u32 + 1));
        }
        b.finish(first_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_round_trip() {
        for (c, s) in [
            (0, "A"),
            (25, "Z"),
            (26, "AA"),
            (701, "ZZ"),
            (702, "AAA"),
            (16_383, "XFD"),
        ] {
            assert_eq!(column_letters(c), s);
            assert_eq!(parse_column_letters(s), Some(c));
            assert_eq!(parse_column_letters(&s.to_lowercase()), Some(c));
        }
        assert_eq!(parse_column_letters("XFE"), None);
        assert_eq!(parse_column_letters("A1"), None);
        assert_eq!(parse_column_letters(""), None);
    }
}
