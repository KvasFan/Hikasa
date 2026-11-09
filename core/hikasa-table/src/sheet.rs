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
}

/// One sheet: ragged rows of packed cells over a text arena (T-5).
#[derive(Debug, Clone)]
pub struct Sheet {
    name: String,
    info: SheetInfo,
    arena: String,
    cells: Vec<Cell>,
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

/// A row of a sheet.
#[derive(Debug, Clone, Copy)]
pub struct RowView<'a> {
    cells: &'a [Cell],
    arena: &'a str,
}

impl<'a> RowView<'a> {
    /// Stored cells (ragged: up to the row's last stored cell).
    pub fn len(&self) -> u32 {
        self.cells.len() as u32
    }

    /// True for a row without cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Value of column `c` (`Empty` past the end).
    pub fn get(&self, c: u32) -> Value<'a> {
        self.cells
            .get(c as usize)
            .map_or(Value::Empty, |cell| cell.value(self.arena))
    }

    /// The packed cell of column `c` (`Cell::EMPTY` past the end).
    pub fn cell(&self, c: u32) -> Cell {
        self.cells.get(c as usize).copied().unwrap_or(Cell::EMPTY)
    }

    /// Every stored value.
    pub fn values(&self) -> impl Iterator<Item = Value<'a>> + 'a {
        let arena = self.arena;
        self.cells.iter().map(move |c| c.value(arena))
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
        self.kinds.get(c as usize).copied().unwrap_or(ColumnKind::Empty)
    }

    /// Formula of data row `r`, column `c`.
    pub fn formula(&self, r: u32, c: u32) -> Option<&str> {
        self.stored_formula(r + self.header_rows(), c)
    }

    /// 1-based label of data row `r`: the first line (text) or the sheet's row number (workbooks).
    pub fn row_label(&self, r: u32) -> u64 {
        self.stored_label(r + self.header_rows())
    }

    /// The regional settings the sheet's text cells are interpreted with.
    pub fn regional(&self) -> &Regional {
        &self.regional
    }

    /// Approximate heap bytes of the sheet (the memory measurements of T-15).
    pub fn heap_bytes(&self) -> usize {
        self.arena.capacity()
            + self.cells.capacity() * std::mem::size_of::<Cell>()
            + (self.rows.capacity() + self.lines.capacity() + self.sheet_rows.capacity()) * 4
            + self.formulas.iter().map(|(_, f)| 16 + f.len()).sum::<usize>()
    }

    /// The merged region covering data row `r`, column `c` (workbooks), in sheet coordinates.
    pub fn merged_region(&self, r: u32, c: u32) -> Option<Region> {
        let row = self.info.origin.0 + r + self.header_rows();
        let col = self.info.origin.1 + c;
        self.info
            .merged
            .iter()
            .find(|g| (g.first_row..=g.last_row).contains(&row) && (g.first_col..=g.last_col).contains(&col))
            .copied()
    }

    /// Adds formulas given in sheet coordinates (the second pass of xlsb, xls and ods). `first_row` is the
    /// sheet row of stored row 0.
    pub(crate) fn add_formulas(&mut self, formulas: Vec<((u32, u32), Box<str>)>, first_row: u32) {
        for ((r, c), f) in formulas {
            let (Some(stored), Some(col)) = (r.checked_sub(first_row), c.checked_sub(self.info.origin.1))
            else {
                continue;
            };
            if stored >= self.stored_count() {
                continue;
            }
            let start = self.rows[stored as usize] as usize;
            let end = self.rows[stored as usize + 1] as usize;
            if start + (col as usize) < end {
                let cell = &mut self.cells[start + col as usize];
                *cell = cell.with_flag(FLAG_FORMULA);
            }
            self.formulas.push(((stored, col), f));
        }
        self.formulas.sort_by(|a, b| a.0.cmp(&b.0));
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
    rows: Vec<u32>,
    lines: Vec<u32>,
    sheet_rows: Vec<u32>,
    formulas: Vec<((u32, u32), Box<str>)>,
    regional: Regional,
    trim_leading: bool,
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
            rows: vec![0],
            lines: Vec::new(),
            sheet_rows: Vec::new(),
            formulas: Vec::new(),
            regional,
            trim_leading: false,
        }
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

    /// Pushes a cell to the current row.
    pub fn push(&mut self, cell: Cell) -> Result<(), TableError> {
        self.check_cells()?;
        self.cells.push(cell);
        Ok(())
    }

    /// Pushes a text cell that is the span `[start, end)` of the arena.
    pub fn push_span(&mut self, start: usize, end: usize) -> Result<(), TableError> {
        self.push(Cell::text(start as u32, (end - start) as u32))
    }

    /// Pushes a text cell whose text is not in the arena (unescaped text, workbook strings).
    pub fn push_text(&mut self, text: &str) -> Result<(), TableError> {
        if text.is_empty() {
            return self.push(Cell::text(0, 0));
        }
        let offset = self.tail.len();
        if offset + text.len() + self.arena.len() > u32::MAX as usize {
            return Err(TableError::TooLarge {
                name: self.name.clone(),
                what: "bytes of text",
                limit: u64::from(u32::MAX),
            });
        }
        self.tail.push_str(text);
        self.push(Cell::text(offset as u32, text.len() as u32).with_flag(FLAG_TAIL))
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

    /// Cells of the current (open) row so far.
    pub fn current_len(&self) -> u32 {
        (self.cells.len() - *self.rows.last().unwrap_or(&0) as usize) as u32
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
        self.formulas.sort_by(|a, b| a.0.cmp(&b.0));
        if self.trim_leading {
            self.trim_leading_columns();
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
            let lead = self.cells[a..b]
                .iter()
                .take_while(|c| c.kind() == CellKind::Empty && !c.has_formula())
                .count() as u32;
            k = k.min(lead);
        }
        if k == 0 || k == u32::MAX {
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

impl Sheet {
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
        for (c, s) in [(0, "A"), (25, "Z"), (26, "AA"), (701, "ZZ"), (702, "AAA"), (16_383, "XFD")] {
            assert_eq!(column_letters(c), s);
            assert_eq!(parse_column_letters(s), Some(c));
            assert_eq!(parse_column_letters(&s.to_lowercase()), Some(c));
        }
        assert_eq!(parse_column_letters("XFE"), None);
        assert_eq!(parse_column_letters("A1"), None);
        assert_eq!(parse_column_letters(""), None);
    }
}
