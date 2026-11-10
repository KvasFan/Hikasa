//! First-line detection (contract §2.5, TB-003) and column-kind detection (T-6, TB-005).

use crate::cell::Value;
use crate::format::FirstLine;
use crate::regional::EXACT_DIGITS;
use crate::sheet::{ColumnKind, Sheet};

/// Rows of the body that the header vote looks at.
const VOTE_ROWS: u32 = 50;

/// Kind of one value: typed cells keep their kind; text is parsed with the sheet's regional settings.
pub(crate) fn value_kind(v: &Value<'_>, sheet: &Sheet) -> ColumnKind {
    match v {
        Value::Empty => ColumnKind::Empty,
        Value::Number(_) => ColumnKind::Number,
        Value::Bool(_) => ColumnKind::Bool,
        Value::DateTime(_) => ColumnKind::DateTime,
        Value::Duration(_) => ColumnKind::Duration,
        Value::Error(_) => ColumnKind::Mixed,
        Value::Text(t) => {
            if t.trim().is_empty() {
                return ColumnKind::Empty;
            }
            let regional = sheet.regional();
            match regional.scan_number(t) {
                // A leading zero (`007`, `01234`) or more than 15 significant digits makes an identifier: the
                // column is text, so `007` and `7` differ (T-6, F2).
                Some(n) if n.leading_zero || n.significant > EXACT_DIGITS => ColumnKind::Text,
                Some(_) => ColumnKind::Number,
                None => {
                    if regional.parse_datetime(t).is_some() {
                        ColumnKind::DateTime
                    } else if regional.parse_bool(t).is_some() {
                        ColumnKind::Bool
                    } else {
                        ColumnKind::Text
                    }
                }
            }
        }
    }
}

/// What a header cell is (§2.5, F5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderCell {
    /// Empty or blank: a column without a name.
    Blank,
    /// Text that is no number, date or bool: a name.
    Label,
    /// A whole number from 1000 to 2999 (`2023`): a name over a numeric column, a value elsewhere.
    Year,
    /// A number, date, duration or bool (typed or as text) of this kind.
    Value(ColumnKind),
    /// An error value.
    Error,
}

/// Classifies a cell of a candidate header row.
pub(crate) fn header_cell(v: &Value<'_>, sheet: &Sheet) -> HeaderCell {
    let year = |n: f64| n.fract() == 0.0 && (1000.0..=2999.0).contains(&n);
    match v {
        Value::Empty => HeaderCell::Blank,
        Value::Error(_) => HeaderCell::Error,
        Value::Number(n) if year(*n) => HeaderCell::Year,
        Value::Text(t) if t.trim().is_empty() => HeaderCell::Blank,
        Value::Text(t) => match sheet.regional().scan_number(t) {
            Some(n) if year(n.value) && !n.leading_zero => HeaderCell::Year,
            _ => match value_kind(v, sheet) {
                ColumnKind::Text => HeaderCell::Label,
                kind => HeaderCell::Value(kind),
            },
        },
        other => HeaderCell::Value(value_kind(other, sheet)),
    }
}

/// The sum of the column votes of the first stored row (`0` without a body to vote on).
fn header_votes(sheet: &Sheet, width: u32) -> i64 {
    let rows = sheet.stored_count();
    if rows < 2 {
        return 0;
    }
    let body_end = rows.min(VOTE_ROWS + 1);
    (0..width).map(|c| column_vote(sheet, c, body_end)).sum()
}

/// True when the first stored row of `sheet` can be the header that the other side of the pair detected (§9.2): it
/// holds no error, and it shares a name with `names` or is all names, blanks and years (at least one name).
pub(crate) fn could_be_header(sheet: &Sheet, names: &[String]) -> bool {
    if sheet.stored_count() == 0 || sheet.stored_row(0).is_empty() {
        return false;
    }
    let row = sheet.stored_row(0);
    let mut label = false;
    let mut only_names = true;
    let mut shared = false;
    for v in row.values() {
        match header_cell(&v, sheet) {
            HeaderCell::Error => return false,
            HeaderCell::Label => label = true,
            HeaderCell::Blank | HeaderCell::Year => {}
            HeaderCell::Value(_) => only_names = false,
        }
        let text = v.display();
        let text = text.trim();
        if !text.is_empty()
            && names
                .iter()
                .any(|n| hikasa_core::case::eq_folded(n.trim(), text))
        {
            shared = true;
        }
    }
    shared || (only_names && label)
}

/// The weighted vote of one column (§2.5, F5): a name over typed values +2, over texts of one other length +1, a
/// name the column repeats −2; a year over numbers 0; a value over values of its kind −2, over another kind +1;
/// blank 0.
fn column_vote(sheet: &Sheet, c: u32, body_end: u32) -> i64 {
    let head = sheet.stored_row(0).get(c);
    let cell = header_cell(&head, sheet);
    if cell == HeaderCell::Blank {
        return 0;
    }
    let header = head.display().trim().to_owned();
    let mut body = ColumnKind::Empty;
    let mut length: Option<usize> = None;
    let mut same_length = true;
    let mut occurs = false;
    for r in 1..body_end {
        let v = sheet.stored_row(r).get(c);
        if v.is_blank() {
            continue;
        }
        body = body.join(value_kind(&v, sheet));
        let text = v.display();
        let text = text.trim();
        let len = text.chars().count();
        match length {
            None => length = Some(len),
            Some(l) if l != len => same_length = false,
            _ => {}
        }
        if text == header {
            occurs = true;
        }
    }
    if body == ColumnKind::Empty {
        return 0;
    }
    let typed = !matches!(body, ColumnKind::Text | ColumnKind::Mixed);
    match cell {
        HeaderCell::Blank => 0,
        HeaderCell::Error => -1,
        _ if occurs => -2,
        HeaderCell::Label if typed => 2,
        HeaderCell::Label if same_length && length != Some(header.chars().count()) => 1,
        HeaderCell::Label => 0,
        HeaderCell::Year if body == ColumnKind::Number => 0,
        HeaderCell::Year => i64::from(typed),
        HeaderCell::Value(kind) if kind == body => -2,
        HeaderCell::Value(_) => i64::from(typed),
    }
}

/// True when the first stored row is a header under `first_line` (§2.5, F5): with Detect, the weighted votes of the
/// columns sum above 0, so blank cells, years and other values are allowed when the other columns vote clearly.
pub(crate) fn detect_header(sheet: &Sheet, first_line: FirstLine, width: u32) -> bool {
    let rows = sheet.stored_count();
    match first_line {
        FirstLine::Names => return rows > 0,
        FirstLine::Data => return false,
        FirstLine::Detect => {}
    }
    if rows < 2 || width == 0 || sheet.stored_row(0).is_empty() {
        return false;
    }
    header_votes(sheet, width) > 0
}

/// Detected kinds of the data columns (header row excluded).
pub(crate) fn column_kinds(sheet: &Sheet, width: u32) -> Vec<ColumnKind> {
    kinds_from(sheet, u32::from(sheet.header().is_some()), width)
}

/// Detected kinds of the columns over the stored rows from `first` on.
pub(crate) fn kinds_from(sheet: &Sheet, first: u32, width: u32) -> Vec<ColumnKind> {
    let mut kinds = vec![ColumnKind::Empty; width as usize];
    for r in first..sheet.stored_count() {
        // Stored cells only: a gap of a sparse row is empty, which changes no kind.
        for (c, v) in sheet.stored_row(r).stored() {
            let Some(kind) = kinds.get_mut(c as usize) else {
                continue;
            };
            if *kind == ColumnKind::Text {
                continue;
            }
            *kind = kind.join(value_kind(&v, sheet));
        }
    }
    kinds
}
