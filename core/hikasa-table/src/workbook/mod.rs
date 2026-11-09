//! Workbooks through `calamine` (contract §3, TB-004, T-14, T-15). The only module that names `calamine`.

use std::io::Cursor;

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

/// Options of the workbook loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadsheetOptions {
    /// Most cells of a dense sheet (xls, ods: the used rectangle) or stored cells of a streamed sheet.
    pub max_cells: u64,
    /// Load formula texts (a second pass for xlsb, xls and ods).
    pub load_formulas: bool,
    /// Load hidden sheets (flagged) — `false` skips them.
    pub load_hidden: bool,
}

impl Default for SpreadsheetOptions {
    fn default() -> SpreadsheetOptions {
        SpreadsheetOptions {
            max_cells: 200_000_000,
            load_formulas: true,
            load_hidden: true,
        }
    }
}

/// Cells between two cancellation checks.
const CHECK_EVERY: u64 = 65_536;

/// The compound-file signature (xls, and encrypted OOXML packages).
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Recognizes a workbook by content, then by name (T-9: an encrypted package is reported, not guessed).
pub fn detect_workbook(bytes: &[u8], name: &str) -> Result<Option<SourceKind>, TableError> {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if bytes.starts_with(&CFB_MAGIC) {
        if contains_utf16(bytes, "EncryptedPackage") || contains_utf16(bytes, "EncryptionInfo") {
            return Err(TableError::Encrypted {
                name: name.to_owned(),
            });
        }
        return Ok(Some(SourceKind::Xls));
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        let has = |needle: &[u8]| memchr::memmem::find(bytes, needle).is_some();
        if has(b"xl/workbook.bin") {
            return Ok(Some(SourceKind::Xlsb));
        }
        if has(b"xl/workbook.xml") {
            return Ok(Some(if ext == "xlsm" || has(b"xl/vbaProject.bin") {
                SourceKind::Xlsm
            } else {
                SourceKind::Xlsx
            }));
        }
        if has(b"application/vnd.oasis.opendocument.spreadsheet") || has(b"content.xml") {
            return Ok(Some(SourceKind::Ods));
        }
        return Ok(None);
    }
    Ok(None)
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
    let kind = match detect_workbook(bytes, name)? {
        Some(kind) => kind,
        None => {
            return Err(TableError::Corrupt {
                name: name.to_owned(),
                message: "not a workbook (xlsx, xlsm, xlsb, xls or ods)".to_owned(),
            });
        }
    };
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
    };
    let mut warnings = Vec::new();
    let sheets = match kind {
        SourceKind::Xlsx | SourceKind::Xlsm => {
            let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).map_err(|e| corrupt(&e))?;
            load_xlsx(&mut wb, &ctx, &mut warnings)?
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
}

impl Ctx<'_> {
    fn too_large(&self) -> TableError {
        TableError::TooLarge {
            name: self.name.to_owned(),
            what: "cells",
            limit: self.opts.max_cells,
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
                        "{}: sheet `{}` is not a worksheet ({:?})",
                        ctx.name, s.name, s.typ
                    ),
                )
                .in_sheet(&s.name),
            );
            continue;
        }
        if hidden && !ctx.opts.load_hidden {
            continue;
        }
        out.push((s.name.clone(), hidden));
    }
    out
}

/// Builds rows from cells that arrive in row-major order with absolute positions.
struct RowFeed {
    builder: SheetBuilder,
    /// Absolute row of the open row (`None` before the first non-empty cell).
    row: Option<u32>,
    first_row: u32,
    cells: u64,
}

impl RowFeed {
    fn new(name: &str, ctx: &Ctx<'_>, hidden: bool, date_1904: bool) -> RowFeed {
        let mut builder = SheetBuilder::new(name, String::new(), ctx.regional);
        builder.info_mut().hidden = hidden;
        builder.info_mut().date_1904 = date_1904;
        builder.set_trim_leading_columns();
        RowFeed {
            builder,
            row: None,
            first_row: 0,
            cells: 0,
        }
    }

    /// Positions the feed at `(row, col)`: closes rows up to `row` and pads with empty cells up to `col`.
    fn seek(&mut self, row: u32, col: u32, ctx: &Ctx<'_>) -> Result<(), TableError> {
        match self.row {
            None => {
                self.row = Some(row);
                self.first_row = row;
            }
            Some(open) if row > open => {
                for _ in open..row {
                    self.builder.end_row(None);
                }
                self.row = Some(row);
            }
            Some(open) if row < open => {
                return Err(TableError::Corrupt {
                    name: ctx.name.to_owned(),
                    message: format!(
                        "cells out of order (row {} after row {})",
                        row + 1,
                        open + 1
                    ),
                });
            }
            _ => {}
        }
        let have = self.builder.current_len();
        if col < have {
            return Err(TableError::Corrupt {
                name: ctx.name.to_owned(),
                message: format!("cells out of order in row {}", row + 1),
            });
        }
        for _ in have..col {
            self.builder.push(Cell::EMPTY)?;
        }
        self.cells += u64::from(col - have) + 1;
        if self.cells > ctx.opts.max_cells {
            return Err(ctx.too_large());
        }
        if self.cells.is_multiple_of(CHECK_EVERY) {
            ctx.cancel.check()?;
        }
        Ok(())
    }

    fn finish(mut self, merged: Vec<Region>, ctx: &Ctx<'_>) -> Sheet {
        if self.row.is_some() {
            self.builder.end_row(None);
        }
        self.builder.info_mut().origin = (self.first_row, 0);
        self.builder.info_mut().merged = merged;
        self.builder.finish(ctx.first_line)
    }
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

/// A serial date-time (T-14): below 1 a time of day on 1899-12-30; else the workbook's epoch, with the 1900
/// epoch's fictitious 1900-02-29 mapped to 1900-02-28. `None` outside 0001–9999.
pub(crate) fn serial_to_ms(serial: f64, date_1904: bool) -> Option<i64> {
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let days = serial.floor();
    let frac_ms = ((serial - days) * 86_400_000.0).round() as i64;
    let d = days as i64;
    let date = if d == 0 {
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

fn push_ref(b: &mut SheetBuilder, v: &DataRef<'_>, date_1904: bool) -> Result<(), TableError> {
    match v {
        DataRef::Int(i) => b.push(Cell::number(*i as f64)),
        DataRef::Float(f) => b.push(Cell::number(*f)),
        DataRef::String(s) => push_str(b, s),
        DataRef::SharedString(s) => push_str(b, s),
        DataRef::Bool(v) => b.push(Cell::bool(*v)),
        DataRef::DateTime(dt) => b.push(datetime_cell(dt, date_1904)),
        DataRef::DateTimeIso(s) => match iso_datetime_cell(s) {
            Some(c) => b.push(c),
            None => push_str(b, s),
        },
        DataRef::DurationIso(s) => match iso_duration_ms(s) {
            Some(ms) => b.push(Cell::duration_ms(ms)),
            None => push_str(b, s),
        },
        DataRef::Error(e) => b.push(Cell::error(error_code(e))),
        DataRef::Empty => b.push(Cell::EMPTY),
    }
}

fn push_data(b: &mut SheetBuilder, v: &Data) -> Result<(), TableError> {
    match v {
        Data::Int(i) => b.push(Cell::number(*i as f64)),
        Data::Float(f) => b.push(Cell::number(*f)),
        Data::String(s) => push_str(b, s),
        Data::Bool(v) => b.push(Cell::bool(*v)),
        // The epoch is inside the value; calamine's own conversion knows it (T-14 below 1 handled by serial).
        Data::DateTime(dt) => {
            let serial = dt.as_f64();
            if dt.is_duration() {
                b.push(Cell::duration_ms((serial * 86_400_000.0).round() as i64))
            } else if serial < 1.0 {
                b.push(datetime_cell(dt, false))
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
                    Some((date, time)) => b.push(Cell::date_time_ms(datetime_to_ms(
                        DateTime::from_parts(date, time),
                    ))),
                    None => b.push(Cell::number(serial)),
                }
            }
        }
        Data::DateTimeIso(s) => match iso_datetime_cell(s) {
            Some(c) => b.push(c),
            None => push_str(b, s),
        },
        Data::DurationIso(s) => match iso_duration_ms(s) {
            Some(ms) => b.push(Cell::duration_ms(ms)),
            None => push_str(b, s),
        },
        Data::Error(e) => b.push(Cell::error(error_code(e))),
        Data::Empty => b.push(Cell::EMPTY),
    }
}

fn push_str(b: &mut SheetBuilder, s: &str) -> Result<(), TableError> {
    if s.is_empty() {
        b.push(Cell::EMPTY)
    } else {
        b.push_text(s)
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
        {
            let mut reader = match wb.worksheet_cells_reader(&name) {
                Ok(r) => r,
                Err(calamine::XlsxError::NotAWorksheet(typ)) => {
                    warnings.push(
                        TableWarning::new(
                            TableWarningCode::SheetSkipped,
                            format!("{}: sheet `{name}` is not a worksheet ({typ})", ctx.name),
                        )
                        .in_sheet(&name),
                    );
                    continue;
                }
                Err(e) => {
                    return Err(TableError::Corrupt {
                        name: ctx.name.to_owned(),
                        message: format!("sheet `{name}`: {e}"),
                    });
                }
            };
            loop {
                let next = reader
                    .next_cell_with_formula()
                    .map_err(|e| TableError::Corrupt {
                        name: ctx.name.to_owned(),
                        message: format!("sheet `{name}`: {e}"),
                    })?;
                let Some(cell) = next else { break };
                let formula = cell.formula.filter(|f| !f.is_empty());
                if is_empty_ref(&cell.value) && formula.is_none() {
                    continue;
                }
                let (row, col) = cell.pos;
                feed.seek(row, col, ctx)?;
                push_ref(&mut feed.builder, &cell.value, date_1904)?;
                if let Some(f) = formula {
                    if is_empty_ref(&cell.value) {
                        no_value += 1;
                    }
                    if ctx.opts.load_formulas {
                        feed.builder.set_formula(&f);
                    }
                }
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
        sheets.push(feed.finish(merged, ctx));
    }
    Ok(sheets)
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
        {
            let mut reader = wb
                .worksheet_cells_reader(&name)
                .map_err(|e| TableError::Corrupt {
                    name: ctx.name.to_owned(),
                    message: format!("sheet `{name}`: {e}"),
                })?;
            while let Some(cell) = reader.next_cell().map_err(|e| TableError::Corrupt {
                name: ctx.name.to_owned(),
                message: format!("sheet `{name}`: {e}"),
            })? {
                if is_empty_ref(cell.get_value()) {
                    continue;
                }
                let (row, col) = cell.get_position();
                feed.seek(row, col, ctx)?;
                push_ref(&mut feed.builder, cell.get_value(), date_1904)?;
            }
        }
        let formulas = if ctx.opts.load_formulas {
            wb.worksheet_formula(&name).ok()
        } else {
            None
        };
        let first_row = feed.first_row;
        let mut sheet = feed.finish(Vec::new(), ctx);
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
                    feed.seek(r0 + r as u32, c0 + c as u32, ctx)?;
                    push_data(&mut feed.builder, v)?;
                }
            }
        }
        let first_row = feed.first_row;
        let mut sheet = feed.finish(merged, ctx);
        if ctx.opts.load_formulas
            && let Ok(f) = wb.worksheet_formula(&name)
        {
            sheet.add_formulas(formulas_of(&f), first_row);
        }
        sheets.push(sheet);
    }
    Ok(sheets)
}
