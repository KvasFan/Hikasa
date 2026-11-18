//! Workbooks through `calamine` (contract §3, TB-004, T-14, T-15). The only module that names `calamine`.

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};

use calamine::{
    CellErrorType, Data, DataRef, Dimensions, ExcelDateTime, Ods, Range, Reader, SheetType,
    SheetVisible, Xls, XlsError, Xlsb, Xlsx,
};
use hikasa_core::CancelToken;
use jiff::civil::{Date, DateTime, Time};

use crate::cell::{Cell, CellError, date_to_ms, datetime_to_ms};
use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::format::{FirstLine, TableFormat, TableKind};
use crate::regional::Regional;
use crate::sheet::{Region, Sheet, SheetBuilder, SourceKind, Workbook};

mod hidden;
mod package;

/// Options of the workbook loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadsheetOptions {
    /// Most cells of a dense sheet (xls, ods: the used rectangle) or stored cells of a streamed sheet.
    pub max_cells: u64,
    /// Most bytes of cell text of the whole workbook, every shared string counted once per sheet that uses it
    /// (F3). `None`: 64 × the input's size, at least 1 GiB.
    pub max_text_bytes: Option<u64>,
    /// Load formula texts (a second pass for xlsb, xls and ods).
    pub load_formulas: bool,
    /// Load hidden sheets (flagged) — `false` skips them with the warning `sheet_skipped`.
    pub load_hidden: bool,
    /// Read the hidden rows and columns of xlsx and xlsm sheets (`SheetInfo::hidden_rows`, `hidden_columns`): a
    /// second, parallel pass over each worksheet part (F13).
    pub read_hidden_rows_columns: bool,
}

impl Default for SpreadsheetOptions {
    fn default() -> SpreadsheetOptions {
        SpreadsheetOptions {
            max_cells: 200_000_000,
            max_text_bytes: None,
            load_formulas: true,
            load_hidden: true,
            read_hidden_rows_columns: true,
        }
    }
}

impl SpreadsheetOptions {
    /// The text budget for an input of `input` bytes.
    pub fn text_budget(&self, input: u64) -> u64 {
        self.max_text_bytes
            .unwrap_or_else(|| input.saturating_mul(64).max(1 << 30))
    }
}

/// Columns a workbook sheet may span (xlsx and ods: XFD); more only when that many cells are stored.
const MAX_COLUMNS: u64 = 16_384;
/// Rows a workbook sheet may span (xlsx: 1,048,576); more only when a quarter as many cells are stored.
const MAX_ROWS: u64 = 1_048_576;

/// Cells between two cancellation checks.
const CHECK_EVERY: u64 = 65_536;

/// The compound-file signature (xls, and encrypted OOXML packages).
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Recognizes a workbook by content, then by name (T-9: an encrypted package is reported, not guessed). A package
/// is recognized by its directory (F11): a damaged or truncated workbook is an error, a zip holding a workbook or
/// an OpenDocument text is no workbook (`None`).
pub fn detect_workbook(bytes: &[u8], name: &str) -> Result<Option<SourceKind>, TableError> {
    match classify(bytes, name)? {
        Class::Workbook(kind) => Ok(Some(kind)),
        Class::Other(_) | Class::Unknown => Ok(None),
    }
}

/// What a file is.
enum Class {
    Workbook(SourceKind),
    /// A package or compound file that is no workbook, and what it is.
    Other(String),
    /// Not a package.
    Unknown,
}

/// Extensions of the workbook formats.
const WORKBOOK_EXTENSIONS: [&str; 8] = ["xlsx", "xlsm", "xlsb", "xls", "xla", "xlam", "ods", "ots"];

/// First parts of OOXML and ODF packages.
const PACKAGE_PARTS: [&str; 7] = [
    "[Content_Types].xml",
    "_rels/",
    "xl/",
    "docProps/",
    "mimetype",
    "content.xml",
    "META-INF/",
];

fn classify(bytes: &[u8], name: &str) -> Result<Class, TableError> {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if bytes.starts_with(&CFB_MAGIC) {
        if contains_utf16(bytes, "EncryptedPackage") || contains_utf16(bytes, "EncryptionInfo") {
            return Err(TableError::Encrypted {
                name: name.to_owned(),
            });
        }
        return Ok(Class::Workbook(SourceKind::Xls));
    }
    if !package::looks_like_package(bytes) {
        return Ok(Class::Unknown);
    }
    let members = match package::directory(bytes) {
        Ok(m) => m,
        Err(why) => {
            let first = package::first_name(bytes).unwrap_or_default();
            if PACKAGE_PARTS.iter().any(|p| first.starts_with(p))
                || WORKBOOK_EXTENSIONS.contains(&ext)
            {
                return Err(TableError::Corrupt {
                    name: name.to_owned(),
                    message: format!("the workbook is damaged or truncated: {why}"),
                });
            }
            return Ok(Class::Other(format!("a damaged zip archive ({why})")));
        }
    };
    let has = |n: &str| package::find(&members, n).is_some();
    if has("xl/workbook.bin") {
        return Ok(Class::Workbook(SourceKind::Xlsb));
    }
    if has("xl/workbook.xml") {
        return Ok(Class::Workbook(
            if ext == "xlsm" || ext == "xlam" || has("xl/vbaProject.bin") {
                SourceKind::Xlsm
            } else {
                SourceKind::Xlsx
            },
        ));
    }
    if let Some(m) = package::find(&members, "mimetype")
        && let Some(data) = package::stored(bytes, m)
    {
        let mime = String::from_utf8_lossy(data);
        let mime = mime.trim();
        if mime.starts_with("application/vnd.oasis.opendocument.spreadsheet") {
            return Ok(Class::Workbook(SourceKind::Ods));
        }
        return Ok(Class::Other(format!(
            "an OpenDocument file of type `{mime}`, not a spreadsheet"
        )));
    }
    if has("content.xml") && (ext == "ods" || ext == "ots") {
        return Ok(Class::Workbook(SourceKind::Ods));
    }
    let names: Vec<&str> = members.iter().take(3).map(|m| m.name.as_str()).collect();
    Ok(Class::Other(format!(
        "a zip archive of {} members ({}{}) without a workbook part",
        members.len(),
        names.join(", "),
        if members.len() > 3 { ", …" } else { "" }
    )))
}

/// Refuses a package whose declared string table exceeds the text budget before the reader inflates it: the
/// declared size bounds what the reader may read (F3).
fn check_string_table(bytes: &[u8], name: &str, budget: u64) -> Result<(), TableError> {
    let Ok(members) = package::directory(bytes) else {
        return Ok(());
    };
    for m in &members {
        if m.name.to_ascii_lowercase().contains("sharedstrings") && m.size > budget {
            return Err(TableError::TooLarge {
                name: name.to_owned(),
                what: "bytes of shared strings",
                limit: budget,
            });
        }
    }
    Ok(())
}

fn contains_utf16(bytes: &[u8], s: &str) -> bool {
    let needle: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    memchr::memmem::find(bytes, &needle).is_some()
}

/// Loads every sheet of a workbook from its bytes.
pub fn load_spreadsheet(
    bytes: &[u8],
    name: &str,
    format: &TableFormat,
    opts: &SpreadsheetOptions,
    cancel: &CancelToken,
) -> Result<Workbook, TableError> {
    let kind = match classify(bytes, name)? {
        Class::Workbook(kind) => kind,
        Class::Other(what) => {
            return Err(TableError::Corrupt {
                name: name.to_owned(),
                message: format!("not a workbook: {what}"),
            });
        }
        Class::Unknown => {
            return Err(TableError::Corrupt {
                name: name.to_owned(),
                message: "not a workbook (xlsx, xlsm, xlsb, xls or ods)".to_owned(),
            });
        }
    };
    if matches!(kind, SourceKind::Xlsx | SourceKind::Xlsm | SourceKind::Xlsb) {
        check_string_table(bytes, name, opts.text_budget(bytes.len() as u64))?;
    }
    let corrupt = |e: &dyn std::fmt::Display| TableError::Corrupt {
        name: name.to_owned(),
        message: e.to_string(),
    };
    let ctx = Ctx {
        name,
        first_line: format.first_line,
        regional: format.regional,
        opts,
        cancel,
        text_budget: opts.text_budget(bytes.len() as u64),
        text_used: AtomicU64::new(0),
    };
    let mut warnings = Vec::new();
    let sheets = match kind {
        SourceKind::Xlsx | SourceKind::Xlsm => {
            let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).map_err(|e| corrupt(&e))?;
            // calamine reports no hidden rows or columns: a parallel pass reads them (F13).
            let (sheets, hidden) = rayon::join(
                || load_xlsx(&mut wb, &ctx, &mut warnings),
                || {
                    if opts.read_hidden_rows_columns {
                        hidden::read(bytes, ctx.text_budget, cancel)
                    } else {
                        Vec::new()
                    }
                },
            );
            let mut sheets = sheets?;
            for (name, h) in hidden {
                if let Some(sheet) = sheets.iter_mut().find(|s| s.name() == name) {
                    sheet.info_mut().hidden_rows = h.rows;
                    sheet.info_mut().hidden_columns = h.cols;
                }
            }
            sheets
        }
        SourceKind::Xlsb => {
            let mut wb: Xlsb<_> = Xlsb::new(Cursor::new(bytes)).map_err(|e| corrupt(&e))?;
            load_xlsb(&mut wb, &ctx, &mut warnings)?
        }
        SourceKind::Xls => {
            let mut wb: Xls<_> = Xls::new(Cursor::new(bytes)).map_err(|e| match e {
                XlsError::Password => TableError::Encrypted {
                    name: name.to_owned(),
                },
                other => corrupt(&other),
            })?;
            load_dense(&mut wb, &ctx, &mut warnings, |wb, sheet| {
                wb.merge_cells_by_sheet_name(sheet).unwrap_or_default()
            })?
        }
        SourceKind::Ods => {
            let mut wb: Ods<_> = Ods::new(Cursor::new(bytes)).map_err(|e| corrupt(&e))?;
            load_dense(&mut wb, &ctx, &mut warnings, |_, _| Vec::new())?
        }
        SourceKind::Delimited | SourceKind::Fixed => Vec::new(),
    };
    Ok(Workbook {
        sheets,
        source: kind,
        detected: TableFormat {
            kind: TableKind::Spreadsheet,
            ..format.clone()
        },
        warnings,
        encoding: None,
    })
}

struct Ctx<'a> {
    name: &'a str,
    first_line: FirstLine,
    regional: Regional,
    opts: &'a SpreadsheetOptions,
    cancel: &'a CancelToken,
    /// The workbook's text budget (F3) and the bytes the finished sheets hold.
    text_budget: u64,
    text_used: AtomicU64,
}

impl Ctx<'_> {
    fn too_large(&self) -> TableError {
        self.limit("cells", self.opts.max_cells)
    }

    fn limit(&self, what: &'static str, limit: u64) -> TableError {
        TableError::TooLarge {
            name: self.name.to_owned(),
            what,
            limit,
        }
    }

    fn corrupt(&self, sheet: &str, e: &dyn std::fmt::Display) -> TableError {
        TableError::Corrupt {
            name: self.name.to_owned(),
            message: format!("sheet `{sheet}`: {e}"),
        }
    }
}

/// The sheets to load, with their hidden flag; skipped ones become warnings.
fn worksheets(
    meta: &[calamine::Sheet],
    ctx: &Ctx<'_>,
    warnings: &mut Vec<TableWarning>,
) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for s in meta {
        let hidden = s.visible != SheetVisible::Visible;
        if s.typ != SheetType::WorkSheet {
            warnings.push(
                TableWarning::new(
                    TableWarningCode::SheetSkipped,
                    format!(
                        "{}: sheet `{}` is not a worksheet ({})",
                        ctx.name,
                        s.name,
                        match s.typ {
                            SheetType::DialogSheet => "a dialog sheet",
                            SheetType::MacroSheet => "a macro sheet",
                            SheetType::ChartSheet => "a chart sheet",
                            SheetType::Vba => "a VBA module",
                            _ => "another kind of sheet",
                        }
                    ),
                )
                .in_sheet(&s.name),
            );
            continue;
        }
        if hidden && !ctx.opts.load_hidden {
            warnings.push(
                TableWarning::new(
                    TableWarningCode::SheetSkipped,
                    format!("{}: hidden sheet `{}` skipped", ctx.name, s.name),
                )
                .in_sheet(&s.name),
            );
            continue;
        }
        out.push((s.name.clone(), hidden));
    }
    out
}

/// One cell of a streamed sheet, its text already in the builder's tail.
struct Item {
    row: u32,
    col: u32,
    cell: Cell,
    formula: Option<String>,
}

/// Builds sparse rows from cells with absolute positions (F4, F8): the open row's cells are buffered and sorted
/// by column (a duplicate position keeps its last cell); a row before the open one asks the caller to re-read the
/// sheet sorted.
struct RowFeed {
    builder: SheetBuilder,
    /// Absolute row of the open row (`None` before the first non-empty cell).
    row: Option<u32>,
    first_row: u32,
    cells: u64,
    pending: Vec<(u32, Cell, Option<String>)>,
    ordered: bool,
}

impl RowFeed {
    fn new(name: &str, ctx: &Ctx<'_>, hidden: bool, date_1904: bool) -> RowFeed {
        let mut builder = SheetBuilder::new(name, String::new(), ctx.regional);
        builder.info_mut().hidden = hidden;
        builder.info_mut().date_1904 = date_1904;
        builder.set_trim_leading_columns();
        builder.set_sparse();
        builder.set_text_limit(
            ctx.text_budget
                .saturating_sub(ctx.text_used.load(Ordering::Relaxed)),
        );
        RowFeed {
            builder,
            row: None,
            first_row: 0,
            cells: 0,
            pending: Vec::new(),
            ordered: true,
        }
    }

    /// Adds a cell; `false` when it lies in a row before the open one.
    fn add(&mut self, item: Item, ctx: &Ctx<'_>) -> Result<bool, TableError> {
        match self.row {
            None => {
                self.row = Some(item.row);
                self.first_row = item.row;
            }
            Some(open) if item.row > open => {
                self.flush()?;
                let rows = u64::from(item.row - self.first_row) + 1;
                let most = MAX_ROWS.max(self.cells.saturating_mul(4));
                if rows > most {
                    return Err(ctx.limit("rows", most));
                }
                for _ in open..item.row {
                    self.builder.end_row(None);
                }
                self.row = Some(item.row);
            }
            Some(open) if item.row < open => return Ok(false),
            _ => {}
        }
        let most = MAX_COLUMNS.max(self.cells + 1);
        if u64::from(item.col) >= most {
            return Err(ctx.limit("columns", most));
        }
        if self.pending.last().is_some_and(|p| item.col <= p.0) {
            self.ordered = false;
        }
        self.pending.push((item.col, item.cell, item.formula));
        self.cells += 1;
        if self.cells > ctx.opts.max_cells {
            return Err(ctx.too_large());
        }
        if self.cells.is_multiple_of(CHECK_EVERY) {
            ctx.cancel.check()?;
        }
        Ok(true)
    }

    /// Writes the open row's cells in column order.
    fn flush(&mut self) -> Result<(), TableError> {
        if !self.ordered {
            self.pending.sort_by_key(|p| p.0);
            let mut kept: Vec<(u32, Cell, Option<String>)> = Vec::with_capacity(self.pending.len());
            for p in self.pending.drain(..) {
                if kept.last().is_some_and(|q| q.0 == p.0) {
                    kept.pop();
                }
                kept.push(p);
            }
            self.pending = kept;
            self.ordered = true;
        }
        for (col, cell, formula) in self.pending.drain(..) {
            self.builder.push_at(col, cell)?;
            if let Some(f) = formula {
                self.builder.set_formula(&f);
            }
        }
        Ok(())
    }

    fn finish(mut self, merged: Vec<Region>, ctx: &Ctx<'_>) -> Result<Sheet, TableError> {
        if self.row.is_some() {
            self.flush()?;
            self.builder.end_row(None);
        }
        ctx.text_used
            .fetch_add(self.builder.text_bytes(), Ordering::Relaxed);
        self.builder.info_mut().origin = (self.first_row, 0);
        self.builder.info_mut().merged = merged;
        Ok(self.builder.finish(ctx.first_line))
    }
}

/// Feeds a sheet's cells: streamed (`false` back when a row comes before the open one) or, with `collect`, all
/// cells read first and sorted by position (the fallback of a sheet whose rows are out of order, F8).
fn drive(
    feed: &mut RowFeed,
    ctx: &Ctx<'_>,
    collect: bool,
    next: &mut dyn FnMut(&mut SheetBuilder) -> Result<Option<Item>, TableError>,
) -> Result<bool, TableError> {
    if !collect {
        while let Some(item) = next(&mut feed.builder)? {
            if !feed.add(item, ctx)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let mut all = Vec::new();
    while let Some(item) = next(&mut feed.builder)? {
        all.push(item);
        if all.len() as u64 > ctx.opts.max_cells {
            return Err(ctx.too_large());
        }
        if (all.len() as u64).is_multiple_of(CHECK_EVERY) {
            ctx.cancel.check()?;
        }
    }
    all.sort_by_key(|i| (i.row, i.col));
    for item in all {
        feed.add(item, ctx)?;
    }
    Ok(true)
}

fn region(d: &Dimensions) -> Region {
    Region {
        first_row: d.start.0,
        first_col: d.start.1,
        last_row: d.end.0,
        last_col: d.end.1,
    }
}

fn error_code(e: &CellErrorType) -> CellError {
    match e {
        CellErrorType::Div0 => CellError::Div0,
        CellErrorType::NA => CellError::NA,
        CellErrorType::Name => CellError::Name,
        CellErrorType::Null => CellError::Null,
        CellErrorType::Num => CellError::Num,
        CellErrorType::Ref => CellError::Ref,
        CellErrorType::Value => CellError::Value,
        CellErrorType::GettingData => CellError::Data,
    }
}

/// A serial date-time (T-14): below 1 a time of day on 1899-12-30, except the 1904 epoch's serial 0, which is its
/// first day, 1904-01-01 (as the same date of a 1900 workbook, serial 1462; F10); else the workbook's epoch, with
/// the 1900 epoch's fictitious 1900-02-29 mapped to 1900-02-28. `None` outside 0001–9999.
pub(crate) fn serial_to_ms(serial: f64, date_1904: bool) -> Option<i64> {
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let days = serial.floor();
    let frac_ms = ((serial - days) * 86_400_000.0).round() as i64;
    let d = days as i64;
    let date = if date_1904 && serial == 0.0 {
        Date::constant(1904, 1, 1)
    } else if d == 0 {
        Date::constant(1899, 12, 30)
    } else if date_1904 {
        Date::constant(1904, 1, 1)
            .checked_add(jiff::Span::new().days(d))
            .ok()?
    } else if d < 60 {
        Date::constant(1899, 12, 31)
            .checked_add(jiff::Span::new().days(d))
            .ok()?
    } else if d == 60 {
        // 1900-02-29 does not exist; Excel's serial 60 is mapped to the 28th.
        Date::constant(1900, 2, 28)
    } else {
        Date::constant(1899, 12, 30)
            .checked_add(jiff::Span::new().days(d))
            .ok()?
    };
    Some(date_to_ms(date) + frac_ms)
}

fn datetime_cell(dt: &ExcelDateTime, date_1904: bool) -> Cell {
    if dt.is_duration() {
        return Cell::duration_ms((dt.as_f64() * 86_400_000.0).round() as i64);
    }
    match serial_to_ms(dt.as_f64(), date_1904) {
        Some(ms) => Cell::date_time_ms(ms),
        None => Cell::number(dt.as_f64()),
    }
}

/// ISO 8601 date-times of ods.
fn iso_datetime_cell(s: &str) -> Option<Cell> {
    let r = Regional::default();
    r.parse_datetime(s).map(Cell::date_time_ms).or_else(|| {
        // A bare time `HH:MM:SS` was handled above; jiff for anything else ISO.
        s.parse::<DateTime>()
            .ok()
            .map(|dt| Cell::date_time_ms(datetime_to_ms(dt)))
    })
}

/// ISO 8601 durations of ods (`PT12H30M05.5S`, `P1DT2H`).
fn iso_duration_ms(s: &str) -> Option<i64> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let rest = rest.strip_prefix('P')?;
    let (days_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut ms: f64 = 0.0;
    let mut num = String::new();
    for c in days_part.chars() {
        match c {
            'D' => {
                ms += num.parse::<f64>().ok()? * 86_400_000.0;
                num.clear();
            }
            '0'..='9' | '.' => num.push(c),
            _ => return None,
        }
    }
    if !num.is_empty() {
        return None;
    }
    for c in time_part.chars() {
        let unit = match c {
            'H' => 3_600_000.0,
            'M' => 60_000.0,
            'S' => 1000.0,
            '0'..='9' | '.' | ',' => {
                num.push(if c == ',' { '.' } else { c });
                continue;
            }
            _ => return None,
        };
        ms += num.parse::<f64>().ok()? * unit;
        num.clear();
    }
    if !num.is_empty() {
        return None;
    }
    let ms = ms.round() as i64;
    Some(if neg { -ms } else { ms })
}

fn cell_of_ref(b: &mut SheetBuilder, v: &DataRef<'_>, date_1904: bool) -> Result<Cell, TableError> {
    Ok(match v {
        DataRef::Int(i) => Cell::number(*i as f64),
        DataRef::Float(f) => Cell::number(*f),
        DataRef::String(s) => text_or_empty(b, s)?,
        // One copy per sheet of each string-table entry, whatever the number of cells using it (F3).
        DataRef::SharedString(s) => {
            if s.is_empty() {
                Cell::EMPTY
            } else {
                b.shared_text_cell(s)?
            }
        }
        DataRef::Bool(v) => Cell::bool(*v),
        DataRef::DateTime(dt) => datetime_cell(dt, date_1904),
        DataRef::DateTimeIso(s) => match iso_datetime_cell(s) {
            Some(c) => c,
            None => text_or_empty(b, s)?,
        },
        DataRef::DurationIso(s) => match iso_duration_ms(s) {
            Some(ms) => Cell::duration_ms(ms),
            None => text_or_empty(b, s)?,
        },
        DataRef::Error(e) => Cell::error(error_code(e)),
        DataRef::Empty => Cell::EMPTY,
    })
}

fn cell_of_data(b: &mut SheetBuilder, v: &Data) -> Result<Cell, TableError> {
    Ok(match v {
        Data::Int(i) => Cell::number(*i as f64),
        Data::Float(f) => Cell::number(*f),
        Data::String(s) => text_or_empty(b, s)?,
        Data::Bool(v) => Cell::bool(*v),
        // The epoch is inside the value; calamine's own conversion knows it (T-14 below 1 handled by serial).
        Data::DateTime(dt) => {
            let serial = dt.as_f64();
            if dt.is_duration() {
                Cell::duration_ms((serial * 86_400_000.0).round() as i64)
            } else if serial < 1.0 {
                // The epoch is only known through the conversion: serial 0 of the 1904 epoch is 1904-01-01.
                let date_1904 = serial == 0.0 && dt.to_ymd_hms_milli().0 == 1904;
                datetime_cell(dt, date_1904)
            } else {
                let (y, mo, d, h, mi, s, ms) = dt.to_ymd_hms_milli();
                let (mo, d) = if y == 1900 && mo == 2 && d == 29 {
                    (2, 28)
                } else {
                    (mo, d)
                };
                match Date::new(y as i16, mo as i8, d as i8)
                    .ok()
                    .zip(Time::new(h as i8, mi as i8, s as i8, i32::from(ms) * 1_000_000).ok())
                {
                    Some((date, time)) => {
                        Cell::date_time_ms(datetime_to_ms(DateTime::from_parts(date, time)))
                    }
                    None => Cell::number(serial),
                }
            }
        }
        Data::DateTimeIso(s) => match iso_datetime_cell(s) {
            Some(c) => c,
            None => text_or_empty(b, s)?,
        },
        Data::DurationIso(s) => match iso_duration_ms(s) {
            Some(ms) => Cell::duration_ms(ms),
            None => text_or_empty(b, s)?,
        },
        Data::Error(e) => Cell::error(error_code(e)),
        Data::Empty => Cell::EMPTY,
    })
}

/// Text cells of a workbook: an empty string is an empty cell; an equal text just appended is reused (ods
/// repeats a cell by copying it).
fn text_or_empty(b: &mut SheetBuilder, s: &str) -> Result<Cell, TableError> {
    if s.is_empty() {
        Ok(Cell::EMPTY)
    } else {
        b.text_cell(s)
    }
}

fn is_empty_ref(v: &DataRef<'_>) -> bool {
    match v {
        DataRef::Empty => true,
        DataRef::String(s) => s.is_empty(),
        DataRef::SharedString(s) => s.is_empty(),
        _ => false,
    }
}

/// Reads one xlsx sheet into `feed`; `None` when the part is no worksheet, `Some(false)` when its rows are out
/// of order (re-read with `collect`).
fn read_xlsx_sheet(
    wb: &mut Xlsx<Cursor<&[u8]>>,
    name: &str,
    ctx: &Ctx<'_>,
    feed: &mut RowFeed,
    collect: bool,
    no_value: &mut u64,
) -> Result<Option<bool>, TableError> {
    let date_1904 = wb.has_1904_epoch();
    let mut reader = match wb.worksheet_cells_reader(name) {
        Ok(r) => r,
        Err(calamine::XlsxError::NotAWorksheet(_)) => return Ok(None),
        Err(e) => return Err(ctx.corrupt(name, &e)),
    };
    let mut next = |b: &mut SheetBuilder| -> Result<Option<Item>, TableError> {
        loop {
            let Some(cell) = reader
                .next_cell_with_formula()
                .map_err(|e| ctx.corrupt(name, &e))?
            else {
                return Ok(None);
            };
            let formula = cell.formula.filter(|f| !f.is_empty());
            let empty = is_empty_ref(&cell.value);
            if empty && formula.is_none() {
                continue;
            }
            if empty {
                *no_value += 1;
            }
            let c = cell_of_ref(b, &cell.value, date_1904)?;
            return Ok(Some(Item {
                row: cell.pos.0,
                col: cell.pos.1,
                cell: c,
                formula: formula.filter(|_| ctx.opts.load_formulas),
            }));
        }
    };
    drive(feed, ctx, collect, &mut next).map(Some)
}

fn load_xlsx(
    wb: &mut Xlsx<Cursor<&[u8]>>,
    ctx: &Ctx<'_>,
    warnings: &mut Vec<TableWarning>,
) -> Result<Vec<Sheet>, TableError> {
    let date_1904 = wb.has_1904_epoch();
    let names = worksheets(wb.sheets_metadata(), ctx, warnings);
    let mut sheets = Vec::with_capacity(names.len());
    for (name, hidden) in names {
        ctx.cancel.check()?;
        // Merged regions are optional facts: a sheet whose merge records do not parse still loads.
        let merged: Vec<Region> = wb
            .merge_cells_by_sheet_name(&name)
            .map(|v| v.iter().map(region).collect())
            .unwrap_or_default();
        let mut feed = RowFeed::new(&name, ctx, hidden, date_1904);
        let mut no_value = 0u64;
        match read_xlsx_sheet(wb, &name, ctx, &mut feed, false, &mut no_value)? {
            None => {
                warnings.push(
                    TableWarning::new(
                        TableWarningCode::SheetSkipped,
                        format!("{}: sheet `{name}` is not a worksheet", ctx.name),
                    )
                    .in_sheet(&name),
                );
                continue;
            }
            Some(true) => {}
            Some(false) => {
                // Rows out of order (F8): read the sheet again, every cell first, then sorted.
                feed = RowFeed::new(&name, ctx, hidden, date_1904);
                no_value = 0;
                read_xlsx_sheet(wb, &name, ctx, &mut feed, true, &mut no_value)?;
            }
        }
        if no_value > 0 {
            warnings.push(
                TableWarning::new(
                    TableWarningCode::FormulaWithoutValue,
                    format!("{no_value} formulas of sheet `{name}` have no cached value (compared as empty)"),
                )
                .in_sheet(&name),
            );
        }
        sheets.push(feed.finish(merged, ctx)?);
    }
    Ok(sheets)
}

/// Reads one xlsb sheet into `feed` (see [`read_xlsx_sheet`]).
fn read_xlsb_sheet(
    wb: &mut Xlsb<Cursor<&[u8]>>,
    name: &str,
    ctx: &Ctx<'_>,
    feed: &mut RowFeed,
    collect: bool,
) -> Result<bool, TableError> {
    let date_1904 = wb.has_1904_epoch();
    let mut reader = wb
        .worksheet_cells_reader(name)
        .map_err(|e| ctx.corrupt(name, &e))?;
    let mut next = |b: &mut SheetBuilder| -> Result<Option<Item>, TableError> {
        loop {
            let Some(cell) = reader.next_cell().map_err(|e| ctx.corrupt(name, &e))? else {
                return Ok(None);
            };
            if is_empty_ref(cell.get_value()) {
                continue;
            }
            let (row, col) = cell.get_position();
            let c = cell_of_ref(b, cell.get_value(), date_1904)?;
            return Ok(Some(Item {
                row,
                col,
                cell: c,
                formula: None,
            }));
        }
    };
    drive(feed, ctx, collect, &mut next)
}

fn load_xlsb(
    wb: &mut Xlsb<Cursor<&[u8]>>,
    ctx: &Ctx<'_>,
    warnings: &mut Vec<TableWarning>,
) -> Result<Vec<Sheet>, TableError> {
    let date_1904 = wb.has_1904_epoch();
    let names = worksheets(wb.sheets_metadata(), ctx, warnings);
    let mut sheets = Vec::with_capacity(names.len());
    for (name, hidden) in names {
        ctx.cancel.check()?;
        let mut feed = RowFeed::new(&name, ctx, hidden, date_1904);
        if !read_xlsb_sheet(wb, &name, ctx, &mut feed, false)? {
            feed = RowFeed::new(&name, ctx, hidden, date_1904);
            read_xlsb_sheet(wb, &name, ctx, &mut feed, true)?;
        }
        let formulas = if ctx.opts.load_formulas {
            wb.worksheet_formula(&name).ok()
        } else {
            None
        };
        let first_row = feed.first_row;
        let mut sheet = feed.finish(Vec::new(), ctx)?;
        if let Some(f) = formulas {
            sheet.add_formulas(formulas_of(&f), first_row);
        }
        sheets.push(sheet);
    }
    Ok(sheets)
}

fn formulas_of(range: &Range<String>) -> Vec<((u32, u32), Box<str>)> {
    range
        .used_cells()
        .filter(|(_, _, f)| !f.is_empty())
        .filter_map(|(r, c, f)| {
            let (r0, c0) = range.start()?;
            Some(((r0 + r as u32, c0 + c as u32), f.as_str().into()))
        })
        .collect()
}

fn load_dense<'b, R: Reader<Cursor<&'b [u8]>>>(
    wb: &mut R,
    ctx: &Ctx<'_>,
    warnings: &mut Vec<TableWarning>,
    merged_of: impl Fn(&R, &str) -> Vec<Dimensions>,
) -> Result<Vec<Sheet>, TableError>
where
    R::Error: std::fmt::Display,
{
    let names = worksheets(wb.sheets_metadata(), ctx, warnings);
    let mut sheets = Vec::with_capacity(names.len());
    for (name, hidden) in names {
        ctx.cancel.check()?;
        let range = wb.worksheet_range(&name).map_err(|e| TableError::Corrupt {
            name: ctx.name.to_owned(),
            message: format!("sheet `{name}`: {e}"),
        })?;
        let (h, w) = range.get_size();
        if (h as u64).saturating_mul(w as u64) > ctx.opts.max_cells {
            return Err(ctx.too_large());
        }
        let merged = merged_of(wb, &name).iter().map(region).collect();
        let mut feed = RowFeed::new(&name, ctx, hidden, false);
        if let Some((r0, c0)) = range.start() {
            for (r, row) in range.rows().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    let empty = match v {
                        Data::Empty => true,
                        Data::String(s) => s.is_empty(),
                        _ => false,
                    };
                    if empty {
                        continue;
                    }
                    let cell = cell_of_data(&mut feed.builder, v)?;
                    feed.add(
                        Item {
                            row: r0 + r as u32,
                            col: c0 + c as u32,
                            cell,
                            formula: None,
                        },
                        ctx,
                    )?;
                }
            }
        }
        let first_row = feed.first_row;
        let mut sheet = feed.finish(merged, ctx)?;
        if ctx.opts.load_formulas
            && let Ok(f) = wb.worksheet_formula(&name)
        {
            sheet.add_formulas(formulas_of(&f), first_row);
        }
        sheets.push(sheet);
    }
    Ok(sheets)
}
